# Borrowing algorithms from other fields for the PCB auto-placer and autorouter

This report has 28 ideas, ordered roughly by how promising they are, and a top 5 at the end. I checked the references marked ✓ with a web search: the paper exists and the venue and year are right. The unmarked references are standard ones I'm confident of but didn't check. Anything marked (?) is uncertain. V/E means value/effort, each rated H/M/L.

Two notes on ratings and tags:
- Value is judged against the pain-point list: P1 pour shredding, P2 BGA escape, P3 no legal two-sided placement, P4 wirelength-optimal placement isn't routable, P5 4-layer runtime, P6 diff pairs, length matching and swap, P7 infeasible vs. gave up, and which net yields.
- Several ideas share machinery. #4, #5 and #24 are one "traffic equilibrium" stack. #2, #6, #11 and #15 are one "local exact endgame" stack.

---

## 1. Pour topology as an exact, incremental cost term (from digital topology, Alexander duality and the game of Hex)

**Origin and references:**
- Hex duality: one player connects if and only if the other player's pieces fail to form a blocking chain (Gale 1979, *Amer. Math. Monthly*, "The game of Hex and the Brouwer fixed-point theorem").
- Digital topology and Euler numbers: Kong & Rosenfeld 1989, *CVGIP*, "Digital topology: introduction and survey".
- Gray 1971, the local "bit-quad" Euler number.
- Alexander duality, where the number of bounded holes in the complement of a planar set K equals b1(K). This is a textbook result.

**The analogy made precise:**
- Per copper layer, let K be the union of four things: foreign copper inflated by clearance, keepouts, the pads of other nets, and the board outline treated as one ring.
- The pour is the complement of K inside the board. So the number of pour components equals b1(K) − (number of empty or enclosed obstacle holes).
- b1 = b0 − χ. The Euler characteristic χ of a rasterised K changes locally: adding one lattice cell changes χ by an amount computable from its 3×3 neighbourhood in O(1). b0 is tracked with union-find, because adding cells only merges clusters.
- So each A* step can know exactly whether it closes a new enclosed pour region, at O(α) cost.

**Corollary you can use before routing.** If both endpoints of a net on layer L already touch the same obstacle cluster, then any route of it on L must enclose a region. The choice is then only which side gets enclosed and whether that side holds a stitching via or a pour-net pad. Otherwise you must change layer.

**Multi-layer version.** Build a pour-net graph. Its nodes are the per-layer pour components, and its edges are stitching vias and through-hole pads of the pour net. A new enclosed region costs nothing if it contains a stitching element. If it doesn't, charge a penalty, or immediately try to insert a stitching via inside it as a cheap repair.

**Addresses:** P1, directly.

**Why it beats the current approach.** Today the pour is repaired after the fact, or priced diffusively; the pressure fields worked only as prices. This is an *exact* topological invariant, available per step, and it separates harmless cuts (which only thin the pour) from fatal ones (which create islands).

**Risks:**
- The Δb0 for a step depends on which clusters the partial path has already touched, which makes the cost path-dependent.
- Options for handling it:
  - Carry a small "first cluster touched" tag in the A* label. This is exact for the common case of two touches.
  - Or apply the rule as a post-route check plus a penalty on history cost.
- Rip-up deletes cells, and union-find can't un-merge. Rebuild per iteration (cheap at this scale) or use a dynamic-connectivity structure.
- Thin necks that DRC would kill need the raster to be inflated by the pour's minimum width, not just by clearance.

**Cheap first experiment.** Implement the per-layer Euler and union-find monitor offline. Log the "island-creating" steps on the boards that stall at 85–90 %, and check whether a few nets cause most of the islands. Then add a penalty of ∞ or H, applied only when the enclosed region contains no stitching element.

**V/E:** H/L-M.

---

## 2. ALNS + MAPF-LNS2 repair loop around the existing rip-up and reroute (from vehicle routing and multi-agent path finding)

**References:**
- Ropke & Pisinger 2006, *Transp. Sci.*, "An adaptive large neighborhood search heuristic for the pickup and delivery problem with time windows".
- Li, Chen, Harabor, Stuckey, Koenig, AAAI 2022, "MAPF-LNS2: Fast repairing for MAPF via LNS" ✓. Code: github.com/Jiaoyang-Li/MAPF-LNS2.
- Potvin & Rousseau 1993, regret insertion.

**The analogy made precise:**
- Agent = net. Collision = shared lattice resource or DRC violation.
- LNS2 starts from colliding paths. It repeatedly destroys a neighbourhood, then replans it with a prioritised planner whose objective is lexicographic: first the number of collisions, then length.
- The neighbourhood operators are:
  - collision-graph connected component;
  - random walk along a blocked agent's path, collecting who blocks it;
  - map region.
- ALNS adds three things: operator weights updated by success (a multi-armed bandit), simulated-annealing acceptance, and *regret-k* repair. Regret-k inserts first the net whose best option beats its second-best by the most.

**Addresses:** the 85–90 % stall (P1 and general density); P7, which net yields.

**Why it beats the current approach.** PathFinder negotiates *all* nets with smooth prices, and late in the run the history terms saturate and it oscillates. LNS2's objective is the discrete collision count, so it focuses on tiny conflict sets. The bandit learns which destroy operator works on *this* board: for example, a "pour-island region" operator (see #1) versus a "BGA window" operator. Regret ordering is a principled answer to "who goes first" at bottlenecks.

**Risks:**
- MAPF agents are point-sized and time-extended. Nets are Steiner trees with width, so a "collision" covers many cells, and you should count conflicting net pairs, not cells.
- ALNS tuning noise.

**Cheap first experiment.** Keep PathFinder for the first N iterations. When overflow stops falling, switch to an LNS2 loop with 3 destroy operators (conflict component, region window, random), regret-2 repair, and weights ρ = 0.1. Measure completion on the stalled 2-layer boards.

**V/E:** H/L.

---

## 3. Placement legalisation as irregular nesting with overlap minimisation (from garment and sheet-metal cutting)

**References:**
- Egeblad, Nielsen, Odgaard 2007, *EJOR* 183, "Fast neighborhood search for two- and three-dimensional nesting problems" ✓. It uses exact 1D translation that minimises overlap, plus guided local search (GLS).
- Gardeyn, Vanden Berghe, Wauters 2025, "An open-source heuristic to reboot 2D nesting research" (sparrow, arXiv:2509.13329) ✓. It is built on **jagua-rs**, a Rust collision-detection engine for irregular cutting and packing ✓.
- Imamichi, Yagiura, Nagamochi 2009, *Discrete Optimization*: iterated local search with nonlinear programming for separation.

**The analogy made precise:**
- Parts (polygons) = courtyards, inflated by a per-class routing margin.
- The sheet = the board outline, one sheet per side.
- The sparrow and GLS pattern turns optimisation into a sequence of *feasibility* problems: shrink the strip, then drive the weighted pairwise overlap to 0.
- GLS pair weights grow on pairs that keep overlapping. This is exactly PathFinder's history cost, applied to placement.
- For PCB there is no strip to shrink. Instead, shrink a *routing-margin inflation factor* upward: maximise the margin at which a zero-overlap placement still exists. Side choice (top or bottom) and rotation become discrete moves.

**Addresses:** P3. Also P4, because the margin can be made per region and congestion-aware.

**Why it beats the current approach.** Simulated annealing on a density-smoothed ePlace output isn't built for *hard* packing near jamming. Nesting heuristics are the state of the art exactly at 85–95 % area utilisation with irregular shapes. The implementation already exists in Rust (jagua-rs), so it may be embeddable or at least usable as a reference.

**Risks:**
- Nesting ignores wirelength. Add a displacement-from-analytic-position term to the overlap objective, as Egeblad's objective allows extra terms.
- Keepout and height rules on two-sided boards need extra constraint types.

**Cheap first experiment.** Feed the ePlace result for a failing two-sided board to jagua-rs or a sparrow-like loop. The objective is overlap plus λ·|displacement|², with 90° rotations and a side flip for parts that are allowed to flip. Check whether a legal placement is found where the SA legaliser fails.

**V/E:** H/M.

---

## 4. Solve the fractional routing relaxation with traffic-assignment algorithms and use the duals (from transportation equilibrium and resource sharing)

**References:**
- Transportation equilibrium:
  - Bar-Gera 2010, *TR-B* 44, "Traffic assignment by paired alternative segments" (TAPAS) ✓.
  - Dial 2006, *TR-B*, "Algorithm B", a bush-based user-equilibrium method.
- Max-concurrent-flow / resource sharing:
  - Garg & Könemann 2007, *SIAM J. Comput.*, multiplicative-weights max-concurrent flow.
  - Müller, Radke, Vygen 2011, *Math. Prog. Comp.*, "Faster min-max resource sharing in theory and practice" (BonnRoute).
  - Albrecht 2001, *IEEE TCAD*.

**The analogy made precise:**
- Origin–destination pairs = two-pin decompositions of nets, or Steiner-tree "bushes".
- Link latency = price of a lattice or tile edge.
- PathFinder is a heuristic tâtonnement toward a congested equilibrium. Bush-based user-equilibrium and resource-sharing algorithms converge orders of magnitude faster and return **dual length functions** y_e.

**Addresses:** P7, P4, and better initial history costs for P1 and P5.

**Why it beats the current approach.** Weak duality gives a *certificate*: if Σ_nets demand·dist_y(net) > Σ_e cap_e·y_e for some y, then the tile graph has no feasible routing at the fractional level. It also names the saturated cut, meaning which nets must yield or change layer. That is the "infeasible vs. gave up" signal, at least at the coarse level, and it converts to capacity-driven placement feedback.

**Risks:**
- Fractional feasibility doesn't imply integral feasibility, so the certificate works in one direction only: it proves "infeasible", never "feasible".
- Tile capacities for 45° tracks with per-class widths are approximate.

**Cheap first experiment.** On the existing coarse tile graph, run about 200 multiplicative-weight rounds of max-concurrent flow. If the concurrent-flow value λ* < 1, report the edges with top y_e as the provable bottleneck. Correlate λ* with final completion across the test boards.

**V/E:** H/M.

---

## 5. Placement gradients from equilibrium sensitivity: routing as the follower in a Stackelberg game (from continuous network design in transportation)

**References:**
- Tobin & Friesz 1988, *Transp. Sci.* 22(4), "Sensitivity analysis for equilibrium network flow" ✓.
- Fosgerau, Frejinger, Karlström 2013, *TR-B* 56, recursive logit ✓.
- Dial 1971, *Transp. Res.*, STOCH multipath loading.

**The analogy made precise:**
- The leader (placement) moves pins. The follower (routing) reaches equilibrium.
- By the envelope theorem, at equilibrium d(total congestion cost)/d(pin position) is roughly the gradient of that net's **price-weighted distance field** at the pin.
- That field is already computed during the #4 solve. So you get a "congestion force" per pin with no extra routing.
- For smoothness, replace hard shortest paths with a **softmin, recursive-logit distance**: V = −τ·log Σ exp(−cost/τ). This is solvable as a linear system z = Mz + b on the tile graph. Expected flow is ∂V/∂cost, so it is differentiable everywhere.
- This is the diffusion field you already tried, but *exponentiated and directional*. It concentrates flow on near-shortest corridors instead of spreading it isotropically.

**Addresses:** P4, replacing the ad-hoc "nudge congested parts".

**Why it beats the current approach.** It gives a principled gradient that plugs into the ePlace objective as a third term, so there is no expensive router call per evaluation.

**Risks:**
- Equilibrium is non-unique and the gradient is non-smooth. Mitigate with the entropic (logit) regularisation.
- τ tuning.

**Cheap first experiment.** Compute softmin fields per net on the coarse tiles using the #4 prices. Add λ·Σ V_net(pin) to the placer. A/B the final routed completion against the current nudging loop.

**V/E:** H/M. It depends on #4.

---

## 6. Conflict-Based Search with symmetry reasoning as a local exact endgame and infeasibility prover (from multi-agent path finding)

**References:**
- Sharon et al. 2015, *AIJ*, CBS.
- Barer et al. 2014, SoCS, ECBS.
- Li, Ruml, Koenig, AAAI 2021, EECBS.
- Li, Harabor, Stuckey, Ma, Gange, Koenig 2021, *AIJ* 301, "Pairwise symmetry reasoning for MAPF search" ✓. It covers rectangle, corridor and target reasoning. Code: CBSH2-RTC.

**The analogy made precise:**
- Take a window around the residual conflicts, with about 3–12 nets whose outside parts are frozen.
- High level: branch on a conflict ("net A may not use cell set S" vs. "net B may not").
- Low level: single-net A* with the added constraints.
- **Corridor reasoning** is exactly the PCB bottleneck, such as a channel between BGA balls or a gap between pads. Instead of an exponential number of one-cell branches, branch once on "A goes through the corridor, B detours" vs. the reverse.
- **Rectangle reasoning** matches two nets crossing in a region on the same layer.

**Addresses:** P7 (when the tree is exhausted inside the window, you get a *proof* of local infeasibility); P2 (small BGA windows); last-mile completion.

**Why it beats the current approach.** Negotiated congestion can't prove anything and wanders. CBS in small windows is complete and bounded-suboptimal (ECBS with a factor w).

**Risks:**
- Windows with a large Steiner fanout blow up.
- The proof is only relative to the frozen boundary and the lattice discretisation. That matters with 0.01 mm slack, so pair it with #20 or #21.

**Cheap first experiment.** On the 10–15 % unrouted remainder, extract windows around each unresolved conflict (≤8 nets) and run ECBS (w = 1.2) with corridor reasoning. Report counts of "solved", "proved infeasible in window" and "timeout".

**V/E:** H/M.

---

## 7. Ordered bundles and metro-line crossing minimisation: crossings become vias (from graph drawing and transit maps)

**References:**
- Pupyrev, Nachmanson, Bereg, Holroyd, GD 2011, "Edge routing with ordered bundles" ✓.
- Fink & Pupyrev, GD 2013, "Metro-line crossing minimization: hardness, approximations, and tractable cases" ✓.
- Bast, Brosi, Storandt, EuroVis 2020, "Metro maps on octilinear grid graphs" ✓. Code: ad-freiburg/loom, octi.
- Nöllenburg & Wolff 2011, *IEEE TVCG*, mixed-integer programming for metro maps.

**The analogy made precise:**
- Transit lines = nets. The embedded graph = your corridor or tile graph after homotopy assignment.
- The ordering of lines along each edge equals the track order inside a channel.
- A crossing of two lines on a 2-layer board costs a via pair, or a pour cut on the other layer.
- The "periphery condition" (lines ending at a station are ordered to its outside) matches pins sitting at the channel edge.
- Stage one: route with an "ink" term that rewards shared corridors (bundling). Stage two: solve the ordering.

**Addresses:** via count; bus-like dense 2-layer boards (P1, since fewer layer swaps mean fewer pour cuts); a faster detailed stage (P5), because bundles route as one fat net.

**Why it beats the current approach.** Your corridor stage fixes homotopy but not *order*, and order decides crossings. MLCM heuristics are near-optimal and fast.

**Risks:**
- Pins aren't at corridor ends, so lines enter and leave mid-edge.
- Width differs per net class, which breaks the uniform-line assumption. That is only minor for ordering.

**Cheap first experiment.** After corridor assignment, run a greedy MLCM-P order per corridor edge. Feed the order as a soft constraint to detailed routing and count vias against the baseline.

**V/E:** M-H/M.

---

## 8. Layer assignment as an Ising spin-glass ground state: exact on planar graphs (from statistical physics)

**References:**
- Barahona, Grötschel, Jünger, Reinelt 1988, *Operations Research*, "An application of combinatorial optimization to statistical physics and circuit layout design" ✓.
- Pinter 1982/84, "Optimal layer assignment for interconnect" ✓.
- Hadlock 1975, *SIAM J. Comput.* 4, "Finding a maximum cut of a planar graph in polynomial time" ✓.

**The analogy made precise:**
- Fix the 2D (projected) routes. Each wire segment between potential via sites is a spin s ∈ {top, bottom}.
- A via is needed where adjacent segments of one net disagree (a ferromagnetic coupling). Crossing segments of different nets must disagree (antiferromagnetic, weight ∞).
- The pour adds a local field. It rewards putting a segment on the layer where it cuts the pour least, using the #1 cut costs as the field.
- Minimising vias is then max-cut. On planar instances this is solvable exactly via T-join or matching.

**Addresses:** P1 (choose layers to keep one pour intact); via reduction, as an exact alternative to renegotiating vias; also the 4-layer case approximately.

**Why it beats the current approach.** Renegotiating vias is local. This is globally optimal for 2 layers given the projection.

**Risks:**
- Pour fields make it non-planar or non-ferromagnetic in general. Solve with QPBO or a small MIP, since your instances are small.
- Projected routes may be poorly chosen.

**Cheap first experiment.** Take finished 2-layer routes, project them to 2D, build the conflict graph, and solve max-cut exactly with a MIP. Compare the via count and the pour-island count.

**V/E:** M/L-M.

---

## 9. Parallel negotiated routing via deterministic reservations (from parallel algorithms and databases)

**References:**
- Blelloch, Fineman, Gibbons, Shun, PPoPP 2012, "Internally deterministic parallel algorithms can be fast" ✓.
- Niu, Recht, Ré, Wright 2011, NeurIPS, "Hogwild!".
- Gort & Anderson, FPT 2010, "Deterministic multi-core parallel routing for FPGAs" ✓, as prior art in a sister domain.

**The analogy made precise:**
- Each round, all pending nets route *in parallel* against a snapshot of the costs. This is the Jacobi form; negotiated congestion tolerates stale prices, which is the Hogwild insight.
- Each net then does a `priority-write-min(cell, net_priority)` on every cell it uses.
- Nets that win *all* their cells commit. Losers retry next round with updated costs.
- The result is deterministic, bit-reproducible, and needs no locks.

**Addresses:** P5.

**Why it beats the current approach.** Bounding-box batching limits parallelism on dense boards, where boxes all overlap. Reservations only serialise *real* conflicts.

**Risks:**
- Late iterations have long conflict chains, so few nets commit per round. Fall back to sequential under about K pending nets.
- Memory per thread for the A* state.

**Cheap first experiment.** Using Rayon: parallel map over nets → reserve → commit. Measure the commit fraction per round and the wall-clock time on 4-layer boards.

**V/E:** H/L-M.

---

## 10. Road-navigation speedups: ALT landmarks and customisable contraction hierarchies (CCH)

**References:**
- Goldberg & Harrelson, SODA 2005, ALT.
- Dibbelt, Strasser, Wagner 2016, *JEA* 21, "Customizable contraction hierarchies" ✓. It covers metric-independent preprocessing plus fast re-customisation when weights change.

**The analogy made precise:**
- Lattice topology is static and congestion only changes weights. That is exactly the "customise per metric" scenario.
- Your costs are at least the base costs, so landmark distances computed on the *static* obstacle lattice stay admissible heuristics under any congestion. Obstacle-dominated boards, such as BGA fields and connector rows, make the octile heuristic weak.

**Addresses:** P5.

**Why it beats the current approach.** ALT typically cuts A* expansions several-fold when obstacles dominate. CCH gives near-instant queries after re-customisation, which takes milliseconds at your scale.

**Risks:**
- Per-net state breaks CCH: the net's own copper is free, and there are clearance-dependent class lattices. ALT is robust to this; CCH less so.
- Landmark memory scales as layers × cells × k.

**Cheap first experiment.** Use 8–16 landmarks per layer-stack, chosen by farthest-point sampling. Compare the expansion count against octile.

**V/E:** M-H/L.

---

## 11. Dead-end elimination and tree decomposition for discrete choice subproblems (from protein side-chain packing)

**References:**
- Desmet et al. 1992, *Nature*, "The dead-end elimination theorem and its use in protein side-chain positioning".
- Xu 2005, RECOMB, side-chain packing via tree decomposition.

**The analogy made precise:**
- Residue = net, or component. Rotamer = one of k candidate routes (or orientations, or placements).
- Pair energy = conflict or overlap between candidates. The global minimum-energy conformation = the best joint selection.
- The DEE criterion E_i(r) + Σ_j min_s E_ij(r,s) > E_i(t) + Σ_j max_s E_ij(t,s) provably prunes candidate r.
- The interaction graph of nets in a window has low treewidth, so dynamic programming over a tree decomposition is exact.

**Addresses:**
- P7: if every candidate combination conflicts, that proves infeasibility *relative to the pool*.
- Pin/gate swap (P6), where each swap choice is a rotamer.
- Discrete orientation and side legalisation (P3).

**Why it beats the current approach.** It is a cheap, provable pruning step before any MIP or search, and it works well on the few-hundred-choice problems you have.

**Risks:**
- Quality is bounded by the diversity of the candidate pool.
- Pairwise energies miss three-way effects.

**Cheap first experiment.** For each stuck window, generate k = 20 diverse routes per net (penalised k-shortest paths), run DEE, then exact DP. Record the fraction solved and the fraction proved empty.

**V/E:** M-H/M.

---

## 12. Length matching and skew as resource-constrained shortest paths (from crew scheduling and vehicle-routing column generation)

**Reference:** Irnich & Desaulniers 2005, "Shortest path problems with resource constraints", in *Column Generation* (Springer).

**The analogy made precise:**
- A label is (cost, length), with Pareto dominance between labels. The window constraint is L_min ≤ length ≤ L_max.
- The router can *absorb* required length through natural detours before falling back to serpentines, and it knows early when a window is unreachable.
- For diff pairs, the resource is accumulated skew (inner- vs. outer-turn length difference), which must end in [−δ, δ].

**Addresses:** P6.

**Why it beats the current approach.** Post-hoc meanders need free space that dense boards lack. Planning the length jointly reserves that space.

**Risks:**
- Label explosion. Bucket the lengths (e.g., 0.1 mm), and use the upper bound on remaining length for pruning.

**Cheap first experiment.** A 2-resource labelling A* on one layer for a group of about 8 matched nets. Compare the leftover meander area with post-hoc tuning.

**V/E:** M/M.

---

## 13. Diff pairs as a rigid "dumbbell" robot in configuration space (from multi-robot formation planning)

**Reference (general):** Lozano-Pérez 1983, configuration-space planning.

**The analogy made precise:**
- State = (centreline cell, heading in 45° steps, gap-mode). Obstacles are inflated by half the pair width plus the gap.
- Turns carry an intrinsic skew cost, fed into #12's resource.
- Near pads, a "split" mode lets the two traces decouple within a max uncoupled length, just as formation planners allow temporary formation breaks.

**Addresses:** P6.

**Why it beats the current approach.** One A* guarantees coupling and spacing, and negotiation treats the pair as one agent.

**Risks:**
- The pad-entry geometry is fiddly.
- The 3D state space is roughly 8–16 times larger per pair, which is fine for a few pairs.

**Cheap first experiment.** A single-layer USB-pair on a test board, compared against two-net routing plus a coupling penalty.

**V/E:** M/M.

---

## 14. Pin and gate swap as assignment, crossing minimisation and soft transport (from operations research, layered graph drawing and optimal transport)

**References:**
- Bertsekas 1988, *Ann. Oper. Res.*, auction algorithm.
- Eades & Wormald 1994, *Algorithmica*, barycentre heuristic for one-sided crossing minimisation.
- Cuturi 2013, NeurIPS, Sinkhorn.

**The analogy made precise:**
- Swappable pin groups (connector pins, resistor arrays, FPGA I/O banks, equivalent gate inputs) are one side of a bipartite graph; net endpoints are the other.
- Cost = estimated route cost + β × the crossings induced against neighbour order. Crossings among "parallel" fanouts are computed by one-sided crossing minimisation, as in Sugiyama drawing. On 2-layer boards crossings are vias.
- During analytic placement, use a Sinkhorn soft assignment so swaps co-evolve with positions. Round with the Hungarian or auction algorithm at legalisation.

**Addresses:** P6 (pin/gate swap), P4.

**Why it beats the current approach.** You currently have no swap support. Assignment is poly-time and exact per group.

**Risks:**
- Swap-equivalence metadata is not in KiCad netlists by default: symbol pin types or footprint "swap groups" are needed.

**Cheap first experiment.** Connector-pin swap on one board via the Hungarian method on estimated wirelength plus barycentre crossings. Measure the via and completion change.

**V/E:** M/L.

---

## 15. Principled "who yields" at a bottleneck: spill metric plus opportunity-cost bids (from compiler register allocation and auctions)

**References:**
- Chaitin–Briggs register allocation: Briggs, Cooper, Torczon 1994, *TOPLAS*.
- Hack, Grund, Goos, CC 2006, SSA-form allocation.
- Bertsekas auction with ε-scaling.
- Vickrey–Clarke–Groves (VCG) pricing.

**The analogy made precise:**
- Interference graph = nets competing for a corridor or layer. Registers = tracks. Spill = rip or reroute, a via detour, or a layer change.
- Chaitin's spill heuristic: evict the net minimising spill_cost / degree.
- For PCB, spill_cost = regret = (best route avoiding the bottleneck) − (current route), computed by one masked A*. Degree = the number of conflicts that net participates in.
- Auction version: each net bids its regret. The bottleneck's price rises by ε until demand ≤ capacity. With ε-complementary slackness it terminates, unlike PathFinder's history cost.

**Addresses:** P7, the yield decision.

**Why it beats the current approach.** History cost encodes "who lost often" rather than "who loses least by moving".

**Risks:**
- A regret computation per contender costs one A* each. It is fine for contested nets only.

**Cheap first experiment.** At the persistent overflow cells, compute masked-A* regret for each occupant and evict the arg-min of regret / degree. Compare iterations-to-legal.

**V/E:** M-H/L.

---

## 16. Inflate-until-jammed placement, with a density certificate (from granular physics)

**Reference:** Lubachevsky & Stillinger 1990, *J. Stat. Phys.*, "Geometric properties of random disk packings". It inflates particles during event-driven dynamics until jamming.

**The analogy made precise:**
- Start with all courtyards scaled by s = 0.5 at their analytic positions. They are trivially legal.
- Raise s while resolving overlaps with nesting moves (#3) or physical relaxation. Record the jamming scale s*.
- s* < 1 means strong evidence of an infeasible placement (not a proof). The parts that are in contact at jamming form the "force network", which names the culprits.

**Addresses:** P3, P7 (for placement).

**Why it beats the current approach.** It turns "the placer found nothing" into a quantitative margin and a list of jammed clusters. It also works as a homotopy that avoids the local minima of direct legalisation.

**Risks:** Jamming depends on the protocol, so s* is a lower bound on the achievable scale.

**Cheap first experiment.** Inflation loop plus Egeblad translation moves on the failing boards; log s* and the contact graph.

**V/E:** M/L-M.

---

## 17. Seam insertion to open a routing channel (from image retargeting)

**Reference:** Avidan & Shamir 2007, SIGGRAPH, "Seam carving for content-aware image resizing".

**The analogy made precise:**
- Energy map = component density + routed congestion.
- A seam is a monotone, min-energy path across the board, found by dynamic programming.
- To "insert" width w, shift every part on one side of the seam by w, locally within slack.

**Addresses:** P4, as a targeted place↔route coupling move to push routing capacity across a saturated cut, for example one found in #4.

**Why it beats the current approach.** It is a global, coherent, minimal-disruption move. Nudging single parts rarely opens a full channel.

**Risks:** The board outline is fixed, so the shift must be absorbed by slack elsewhere; the seam should pass through low-slack regions and push into high-slack ones.

**Cheap first experiment.** Take the #4 bottleneck edge set, compute the DP seam orthogonal to it, and shift parts via the legaliser. Re-route.

**V/E:** M/L.

---

## 18. Maximum-weight independent sets over candidate positions (from cartographic label placement)

**References:**
- Christensen, Marks, Shieber 1995, *ACM TOG*, "An empirical study of algorithms for point-feature label placement".
- Lamm et al., ALENEX 2019, exact maximum-weight independent set (MWIS) on large sparse graphs (KaMIS).

**The analogy made precise:**
- Label = component. Candidate label slots = sampled placement candidates around the analytic position (with rotation and side).
- Overlap conflicts form a graph, and you choose one candidate per component with maximum total quality.
- KaMIS-style kernelisation solves instances with thousands of vertices exactly.

**Addresses:** P3.

**Why it beats the current approach.** It is global and exact on the candidate pool, where simulated annealing is local and stochastic.

**Risks:**
- "Exactly one per component" is a constraint; handle it with clique-per-component formulations.
- The quality of candidate generation dominates.

**Cheap first experiment.** 30 candidates per part (4 rotations × 2 sides × jittered positions) → conflict graph → MWIS with a KaMIS-like solver or CP-SAT.

**V/E:** M/M.

---

## 19. Precomputed exact-geometry escape tiles with wave-function-collapse-style propagation (from procedural content generation)

**References:**
- Karth & Smith, FDG 2017, "WaveFunctionCollapse is constraint solving in the wild" ✓.
- Merrell 2007, I3D, "Example-based model synthesis".
- Luo & Wong, ASP-DAC 2008, "Ordered escape routing based on Boolean satisfiability" ✓, as the PCB-native baseline.

**The analogy made precise:**
- A BGA field is periodic. Each cell (the gap between 2×2 balls) is a tile with a small alphabet: {0, 1, 2 traces per direction per layer; via-in-cell or none; dog-bone orientation}.
- Compute adjacency compatibility **offline with exact integer geometry** per (pitch, ball, via, width, clearance) tuple. The 0.01 mm slack is then settled once, exactly, and never by lattice search.
- Arc-consistency propagation plus backtracking (the wave-function-collapse loop, i.e. a constraint-satisfaction solver) assigns tiles so that every ball reaches the perimeter.

**Addresses:** P2.

**Why it beats the current approach.** The lattice can't represent 0.01 mm slack robustly, while the tile catalogue encodes it exactly and the search space collapses.

**Risks:**
- Irregular BGAs (depopulated centres, mixed pitch) need more tile types.
- Ordered escape needs global ordering constraints on top.

**Cheap first experiment.** Enumerate the tile catalogue for one 0.8 mm-pitch rule set, solve a full escape with CP-SAT over tiles, and compare with the router.

**V/E:** M-H/M.

---

## 20. Monotone dynamic programming for ordered gap assignment (from typesetting: Knuth–Plass line breaking)

**Reference:** Knuth & Plass 1981, *Software: Practice & Experience*, "Breaking paragraphs into lines".

**The analogy made precise:**
- Words = nets in their required order (ordered escape, bus order). Lines = gaps between balls or pins along a row, each with capacity k_g.
- Badness = the clearance slack consumed, or the detour length.
- A monotone assignment of an ordered sequence to ordered bins is O(n·m) DP, exactly like line breaking.

**Addresses:** P2, and threading buses through pin rows.

**Why it beats the current approach.** It is exact and instant, and it prevents A* from discovering the order infeasibility late.

**Risks:** It is 1D per row; coupling across rows needs iteration or #19.

**Cheap first experiment.** Per BGA ring, assign perimeter nets to gaps by DP and pass the assignment as corridor constraints.

**V/E:** M/L.

---

## 21. Sequential convex trajectory optimisation with exact penalties for slack-critical geometry (from robot motion planning)

**References:**
- Schulman et al. 2014, *IJRR*, "Motion planning with sequential convex optimization and convex collision checking" (TrajOpt).
- Ratliff et al. 2009, ICRA, CHOMP.

**The analogy made precise:**
- Trace = trajectory of vertices. Clearance = signed distance to obstacles, which must be ≥ half-width + clearance.
- Trust-region sequential quadratic programming with ℓ1 exact penalties (not the springs of position-based dynamics).
- Constraints include length (matching), coupling (diff pairs) and 45° angles, via linearised direction constraints.

**Addresses:** P2 (squeeze lattice routes into exact legal positions), P6.

**Why it beats the current approach.** Position-based-dynamics rubber bands converge softly and can't certify a violation. An exact penalty either hits zero violation (legal) or converges to a stationary point with violation > 0. That is not a proof, but it is a strong local signal.

**Risks:** Topology changes aren't possible, since it is a local method.

**Cheap first experiment.** Take DRC-failing BGA escape segments, set up a quadratic program per SQP step with OSQP, and check whether they become legal.

**V/E:** M/M.

---

## 22. Exact C-space tangent and visibility graphs inside escape windows (from robotics)

**Reference:** Lozano-Pérez & Wesley 1979, *CACM*, "An algorithm for planning collision-free paths among polyhedral obstacles".

**The analogy made precise:**
- Inflate obstacles by the Minkowski sum with half-width + clearance, as octagons.
- Shortest paths lie on a tangent graph, computed in i64 nanometres.
- This is *gridless* routing only inside hard windows, spliced to the lattice outside.

**Addresses:** P2.

**Why it beats the current approach.** The lattice step can't resolve 0.01 mm. Exact geometry decides it definitively, which also serves P7 locally.

**Risks:**
- Integrating with negotiated congestion; treat the window as one super-node with discrete path options (feeding #11).

**Cheap first experiment.** A single BGA quadrant with exact octagon C-space, checking which nets are geometrically escapable at all.

**V/E:** M/M.

---

## 23. Pre-route a loopy, width-tapered ground and power skeleton (from leaf venation and vascular networks)

**References:**
- Katifori, Szöllősi, Magnasco 2010, *PRL* 104, "Damage and fluctuations induce loops in optimal transport networks" ✓.
- Runions et al., SIGGRAPH 2005, leaf venation via space colonisation.
- Murray 1926, *PNAS*, Murray's law.

**The analogy made precise:**
- Vein network = pour-net skeleton, sized by Murray-like width scaling.
- Its loops make the network robust to "damage", which here means signal cuts.
- Grow it first from the regulator or connector to all pour-net pads, with redundancy (loops) around dense signal regions, and reserve it at high priority. Signals route in its meshes and the pour then fills.
- Connectivity then holds by construction, and #1 shows which loops are still missing.

**Addresses:** P1.

**Why it beats the current approach.** Today the pour is a leftover. Planning it as a network with explicit redundancy is closer to how humans lay out 2-layer ground grids.

**Risks:** It consumes area and may reduce signal completion. Tune the loop density by fluctuation strength, which is what Katifori's model provides.

**Cheap first experiment.** A space-colonisation skeleton with one loop per k signals crossing, reserved and routed. Measure islands and completion.

**V/E:** M/M.

---

## 24. Marginal-cost tolls: steering PathFinder from user equilibrium toward system optimum (from transportation economics)

**References:**
- Beckmann, McGuire, Winsten 1956, *Studies in the Economics of Transportation*.
- Roughgarden & Tardos 2002, *JACM*, "How bad is selfish routing?".

**The analogy made precise:**
- PathFinder's present cost p(occupancy) is a "latency". Selfish nets converge to Wardrop user equilibrium, which can be Braess-bad (a new via or layer option makes things worse).
- The system optimum needs the Pigouvian toll t(x) + x·t′(x), which charges each net the externality it imposes.

**Addresses:** P1, P4, and the late-stage oscillations.

**Why it beats the current approach.** It is a one-line change to the cost function, and the theory says it cures the price-of-anarchy gap.

**Risks:** With hard capacities the "latency" is a penalty surrogate, so the benefit is empirical.

**Cheap first experiment.** Replace the present-cost increment with its marginal form and A/B the completion rate and iteration count.

**V/E:** M/L.

---

## 25. Graph-cut segmentation with connectivity priors for split planes and pour allocation (from computer vision)

**References:**
- Boykov, Veksler, Zabih 2001, *PAMI*, α-expansion.
- Vicente, Kolmogorov, Rother, CVPR 2008, "Graph cut based image segmentation with connectivity priors" ✓.

**The analogy made precise:**
- Pixels = board cells per layer. Labels = {GND, 3V3, 5V, signal-reserved}.
- Unary costs = pad affinity and signal occupancy. Pairwise costs = boundary length plus a penalty for signals crossing a split (return path).
- The connectivity prior forces each plane label to be one connected region covering its pads.

**Addresses:** P1, and multi-net pours.

**Why it beats the current approach.** It gives a global partition with explicit connectivity instead of fill-then-check.

**Risks:** Connectivity priors are NP-hard; use Vicente's heuristics or seed-path constraints.

**Cheap first experiment.** A 3-label α-expansion on a 4-layer power layer with seed pads; then check connectivity with #1.

**V/E:** M/M.

---

## 26. PIBT and LaCAM as a push-and-shove protocol (from multi-agent path finding)

**References:**
- Okumura, Machida, Défago, Tamura 2022, *AIJ*, "Priority inheritance with backtracking for iterative MAPF" ✓.
- Okumura 2023, AAAI, LaCAM ✓.

**The analogy made precise:**
- When net A needs cells held by B, B *inherits* A's priority and must relocate, recursively, with backtracking if B can't move.
- LaCAM wraps this in a complete search over "configurations", here the joint positions of the nets in a window.

**Addresses:** P7, and the last-mile completion.

**Why it beats the current approach.** Rip-up is destructive and global, while inheritance-based shove is local and cascading.

**Risks:**
- PIBT's power comes from temporal one-step moves, and PCB has no time axis, so the analogy is weaker than for CBS or LNS.
- It is better treated as a heuristic inside #2.

**Cheap first experiment.** Use it as an LNS2 repair operator: a priority-inheritance shove within a window.

**V/E:** L-M/M.

---

## 27. Multi-fidelity surrogate for "router as judge" (from computer experiments and engineering design)

**Reference:** Kennedy & O'Hagan 2000, *Biometrika*, "Predicting the output from a complex computer code when fast approximations are available".

**The analogy made precise:**
- Low fidelity = the #4 fractional λ* and the #5 softmin cost. Medium = coarse-tile routing. High = the full router.
- Co-kriging, or a residual model, learns high minus low and decides when an expensive call is worthwhile (expected-improvement acquisition).

**Addresses:** P4.

**Why it beats the current approach.** It spends router calls only where the cheap proxies are uncertain.

**Risks:** Few samples per board. It needs cross-board features, or else stays per-board Bayesian.

**Cheap first experiment.** Log (λ*, softmin cost, final completion) over existing runs and check rank correlation. If ρ > 0.8, the full surrogate isn't even needed.

**V/E:** M/M.

---

## 28. Physarum with conductance adaptation (from biology) — low priority

**References:**
- Tero, Kobayashi, Nakagaki 2007, *J. Theor. Biol.*
- Tero et al. 2010, *Science*.
- Bonifaci, Mehlhorn, Varma 2012, *J. Theor. Biol.*

**The analogy made precise.** Your resistor network plus the adaptation dD/dt = |Q|^μ − D converges to shortest paths or Steiner-like networks.

**Assessment.**
- You already found that pressure fields are useful only as prices, and Physarum converges slower than #4 or #5 to the same kind of answer.
- The only distinct use is growing the #23 skeleton with loops. Fluctuating-source Physarum yields loops, but #23 does that more directly.

**V/E:** L/L.

---

## Considered and rejected

- Ant colony optimisation: it is dominated by A* with negotiation.
- Railway timetabling: the time axis is essential there, and the clique ILPs amount to #11.
- Wavelength assignment in optical networks: it maps well (wavelength = layer, converter = via), but it is subsumed by #8 and #15.
- Hierarchical A* from games: you already have the tile corridors.
- Jump point search: it breaks under non-uniform costs.
- Origami, knitting, DNA tiling, sports scheduling: no structural fit beyond generic constraint satisfaction.

---

## Top 5

1. **#1 Pour topology via Euler characteristic and union-find (Hex duality).** It is the only idea that targets P1, the worst pain point, with an exact, O(1)-per-step invariant. It separates harmless cuts from fatal ones and turns "island" into a routing cost or an automatic stitching-via repair. The build is a few hundred lines of code and needs no new solver.
2. **#2 ALNS + MAPF-LNS2 repair.** It wraps the existing rip-up and reroute rather than replacing it. It attacks the 85–90 % plateau with a collision-count objective, adaptive neighbourhoods (including a pour-island operator from #1) and regret ordering, and it answers "who yields". The effort is low and there is strong evidence from the MAPF benchmarks.
3. **#3 Nesting (GLS overlap minimisation; sparrow/jagua-rs, in Rust).** P3 is exactly the regime (hard feasibility near jamming, irregular shapes) where nesting heuristics are the state of the art. There is an open Rust implementation to reuse or crib from, and #16's jamming scale gives a free infeasibility signal.
4. **#4 Fractional equilibrium plus dual certificates (and #5 on top of it).** It is the one route to a *provable* "infeasible" (P7) at the corridor level. It gives better-founded prices than history costs, and via the envelope theorem (#5) it supplies placement gradients for P4 almost for free. #15 and #17 consume its bottleneck output.
5. **#9 + #10 Deterministic reservations plus ALT landmarks.** P5 is pure engineering pain. Both are low-risk and orthogonal, so the speedups multiply, and deterministic reservations keep results reproducible, which matters for regression testing a router.

Honourable mentions: #6 (CBS corridor reasoning in windows) is the strongest complement to #2 for both completion and local infeasibility proofs. For P2, combine #19, #20 and #22: exact geometry by construction instead of lattice search.

---

## Sources for the references I checked
- [sparrow: An open-source heuristic to reboot 2D nesting research (arXiv 2509.13329)](https://arxiv.org/abs/2509.13329), [sparrow on GitHub](https://github.com/JeroenGar/sparrow)
- [MAPF-LNS2 on dblp](https://dblp.org/rec/conf/aaai/0001CHSK22.html), [MAPF-LNS2 on GitHub](https://github.com/Jiaoyang-Li/MAPF-LNS2)
- [Pairwise symmetry reasoning for MAPF (arXiv 2103.07116)](https://arxiv.org/abs/2103.07116)
- [Edge Routing with Ordered Bundles (arXiv 1209.4227)](https://arxiv.org/abs/1209.4227)
- [Metro-line crossing minimization (arXiv 1306.2079)](https://arxiv.org/abs/1306.2079)
- [Metro Maps on Octilinear Grid Graphs](https://diglib.eg.org/handle/10.1111/cgf13986), [loom on GitHub](https://github.com/ad-freiburg/loom)
- [Internally deterministic parallel algorithms can be fast](https://dl.acm.org/doi/10.1145/2145816.2145840)
- [Deterministic multi-core parallel routing for FPGAs](https://ieeexplore.ieee.org/document/5681758)
- [Customizable Contraction Hierarchies](https://dl.acm.org/doi/10.1145/2886843)
- [Hadlock 1975, planar max-cut (Google Scholar)](https://scholar.google.com/scholar_lookup?title=Finding+a+maximum+cut+of+a+planar+graph+in+polynomial+time&author=F.+Hadlock&publication_year=1975), [Barahona et al., Operations Research](https://dl.acm.org/doi/10.5555/2804709.2804720), [Pinter, Optimal layer assignment](https://dl.acm.org/citation.cfm?id=2335)
- [Luo & Wong, Ordered escape routing via SAT](https://www.researchgate.net/publication/4327316_Ordered_escape_routing_based_on_Boolean_satisfiability)
- [Katifori et al., Damage and fluctuations induce loops (arXiv 0906.0006)](https://arxiv.org/abs/0906.0006)
- [Egeblad et al., Fast neighborhood search for nesting](https://www.sciencedirect.com/science/article/abs/pii/S037722170600302X)
- [Vicente et al., Graph cut segmentation with connectivity priors](https://www.microsoft.com/en-us/research/publication/graph-cut-based-image-segmentation-with-connectivity-priors/)
- [Tobin & Friesz 1988](https://pubsonline.informs.org/doi/10.1287/trsc.22.4.242)
- [Bar-Gera 2010, TAPAS](https://www.sciencedirect.com/science/article/abs/pii/S0191261509001350)
- [Fosgerau et al. 2013, recursive logit](https://www.sciencedirect.com/science/article/abs/pii/S0191261513001276)
- [Karth & Smith, WFC is constraint solving](https://dl.acm.org/doi/10.1145/3102071.3110566)
- [LaCAM (IJCAI follow-up; references the AAAI 2023 original and PIBT in AIJ 2022)](https://www.ijcai.org/proceedings/2023/0028.pdf)
