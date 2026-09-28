# TCS/OR ideas for the KiCad placer and router

**How references were checked.** I verified these by web search: McMurchie–Ebeling PathFinder is not in that list, but GRIP, OptRouter, Bayless–Hoos–Hu ICCAD'16, Yan–Wong DAC'09, Luo–Wong ASP-DAC'08, Hougardy–Silvanus–Vygen MPC'17, Leiserson–Maley STOC'85, Müller–Radke–Vygen MPC'11, Gort–Anderson FPT'10, Chen–Kajitani–Chan TCAS'83, Ozdal–Wong TCAD'06 and Brenner–Vygen TCAD'04 were all confirmed. The other citations are standard ones I am confident of. The few I am unsure of are marked **(?)**.

**Ratings.** V = value, E = effort, each on a 1–5 scale; 5 means high value or high effort.

**One caveat applies to every certificate below.** The routing lattice is a *restriction* of the real geometry, since it forbids off-grid routes. So "lattice-infeasible" does not mean "board-infeasible". A real proof of infeasibility needs a *relaxation*: capacities that are provable upper bounds. The ideas below keep these two levels separate.

---

## A. Certificates, bounds, and "infeasible or gave up?"

### 1. Farkas / Lagrangian certificates from PathFinder's own prices (V5, E1)
- **Mapping.**
  - Resources r: lattice node-layers, with capacity c_r (usually 1).
  - Net n chooses a tree T from 𝒯_n. The tree consumes its clearance-inflated footprint, including via footprints on all layers.
  - LP relaxation: λ_{n,T} ≥ 0, Σ_T λ_{n,T} = 1, Σ_{n,T} λ_{n,T}[r∈T] ≤ c_r.
- **Farkas lemma.** The system is infeasible iff some y ≥ 0 has Φ(y) = Σ_n min_T y(T) − Σ_r y_r c_r > 0.
- **Cheap version.** Each iteration, plug in y = the current history costs, or history × present. The per-net minima are *almost* free because you already run A*. Two conditions:
  - A* must be exact and unrestricted by the corridor, with a consistent heuristic.
  - For multi-pin nets you need a valid *lower bound* on the Steiner tree cost. Options: the maximum pairwise distance; MST(metric closure)/2 (valid because the Steiner ratio in graphs is ≤ 2); or exact Dijkstra–Steiner for k ≤ 6 (idea 16).
- **Maximising Φ.** Normalise y·c = 1 and maximise Φ by subgradient steps. That is exactly the Lagrangian dual of the feasibility LP.
- **Reading the result:**
  - Φ > 0: provably unroutable *on this lattice*. Stop negotiating; refine the lattice, move parts, or drop a net.
  - Φ ≤ 0 everywhere and the LP is feasible, yet negotiation stalls: the difficulty is integral (crossings or ordering). Switch to exact window solving (idea 7) instead of running more iterations.
- **Guarantees.** Exact duality for the fractional model. The node-capacity-1 lattice LP is weak because half-half splits are allowed, so it certifies only gross infeasibility. It is still free.
- **Risk.** Low. The main trap is using the corridor-restricted A* by mistake, which does not give a valid lower bound.
- **First experiment.** Log Φ(history) per iteration on the stalled 2-layer boards and see whether it ever goes positive.

### 2. Geometric cut certificates (critical cuts and sparsest cut) (V5, E3)
- **Refs.**
  - Leiserson & Maley, "Algorithms for routing and testing routability of planar VLSI layouts", STOC'85.
  - Maley, "Testing homotopic routability under polygonal wiring rules", Algorithmica 1996.
  - Dai, Kong & Sato, "Routability of a rubber-band sketch", DAC'91.
  - Okamura & Seymour, JCTB 1981.
  - Leighton & Rao, JACM 1999.
- **Mapping.**
  - A *cut* Γ is a chain of segments γ_i between obstacles (pads, keepouts, board edge) that splits the board into S and S̄. Obstacle-to-obstacle gaps come from a Delaunay triangulation of the obstacles.
  - Capacity of a segment on layer ℓ: κ(γ,ℓ) = ⌊(|γ| − c_eff)/(w_min + c_min)⌋. Using the minimum width and clearance keeps it an upper bound. For mixed net classes it becomes a knapsack; bound it conservatively.
  - Through vias and multi-layer pads block every layer.
  - Demand D(Γ) = number of nets with pins on both sides. Each Steiner tree crosses Γ at least once, so this is valid for multi-pin nets too.
  - Add the pour net: if it has pads on both sides and must stay connected, it needs at least one "neck" of minimum pour width crossing Γ on some layer.
  - Infeasible if D(Γ) > Σ κ.
- **Finding cuts.** By planar duality, a minimum cut is a shortest path in the obstacle-gap graph from boundary to boundary (edge weight = capacity). For multicommodity demand, solve the max-concurrent-flow LP on the coarse graph (idea 4). The dual lengths, rounded by Leighton–Rao region growing, give a near-sparsest cut.
- **Output a human-readable reason.** For example: "27 nets must cross the 3.1 mm line between U4 and J2. It fits 22 tracks on 2 layers."
- **Guarantees.**
  - The cut condition is necessary only. The fractional flow-cut gap is O(log k); the integral gap is worse.
  - It is sufficient in two special cases:
    - Okamura–Seymour: planar graphs with all terminals on the outer face, Eulerian capacities.
    - Leiserson–Maley: single layer, fixed topology (homotopy). "All critical cuts safe ⇔ routable", including polygonal and 45° rules per Maley'96.
  - The second case gives an exact per-layer geometric check once the lattice has fixed the topology (see idea 3).
- **Replaces.** Nothing; it adds a diagnosis layer. It also feeds Benders cuts to the placer (idea 18).
- **Risk.** Exponentially many cuts. Heuristic generation (LP duals, obstacle pairs) finds violated ones but can't prove none exist. That is fine for a certificate, because one violated cut suffices.
- **First experiment.** Build Delaunay over pad and courtyard obstacles, run a dual shortest path for every pair of board-edge points, and report every cut with D > κ on the stalled boards.

### 3. Geometric realisation by LP compaction, with negative cycles as certificates (V4, E3)
- **Refs.**
  - Liao & Wong, "An algorithm to compact a VLSI symbolic layout with mixed constraints", TCAD 1983.
  - Maley's homotopic compaction work, as above.
- **Mapping.**
  - Fix the topology from the lattice route: which tracks pass through which inter-obstacle gap, in which order, on which layer.
  - Variables: offsets of each segment along its normal. With fixed directions (0/45/90/135°) the constraints stay linear.
  - Clearance constraints (w_i+w_j)/2 + c_ij ≤ x_j − x_i. In 1-D these are difference constraints: solve by Bellman–Ford or longest path. Otherwise a small LP.
  - Objective: maximise the minimum slack, or minimise wirelength.
- **Payoff for pain 2 (0.01 mm slack).** The lattice only needs to get the *topology* right; exact coordinates come from the LP. No more lattice-pitch alignment tricks.
  - If infeasible, the Farkas ray is a positive cycle: an explicit chain "pad–track–track–pad whose widths plus clearances exceed the gap by 7 µm".
- **Risk.** 2-D coupling of 45° segments makes it a general LP rather than a network LP. It is still small per region.
- **First experiment.** Take a BGA escape the lattice fails by one track, and solve the 1-D spacing LP per channel with the topology forced.

---

## B. Global routing by LP decomposition (pains 4, 5, 7)

### 4. Min-max resource sharing (multiplicative weights) as the corridor planner (V5, E3)
- **Refs.**
  - Garg & Könemann, FOCS'98 / SICOMP'07.
  - Albrecht, "Global routing by new approximation algorithms for multicommodity flow", TCAD 2001.
  - Müller, Radke & Vygen, "Faster min-max resource sharing in theory and practice", MPC 2011.
  - Gester et al., "BonnRoute", TODAES 2013.
  - Arora, Hazan & Kale, Theory of Computing 2012 (MWU survey).
  - Awerbuch, Azar & Plotkin, FOCS'93 (exponential costs, online setting).
- **Mapping.**
  - Customers: nets.
  - Block of each net: the convex hull of its Steiner trees in the 3-D tile graph (tiles × layers plus via edges).
  - Resources:
    - tile-boundary track capacity per layer, from the geometric upper bound in idea 2;
    - via capacity per tile;
    - a "pour keep-free" resource per tile (pour area left ≥ threshold, so the pour stays connected);
    - total wirelength and total vias as extra "customers".
  - Oracle: an approximate minimum-price Steiner tree. The σ-approximate oracle gives a σ(1+ω) approximation of the min-max congestion λ*.
  - Prices p_r = exp(α·u_r/c_r). There are O(ω⁻² log|R|) oracle calls per net.
- **Why it fits:**
  - **Parallelism.** Within a phase, nets are priced against stale prices, which the theory tolerates. BonnRoute runs this deterministically in parallel. That directly attacks pain 5, where long nets serialise PathFinder.
  - **Certificate.** If λ* > 1 on a relaxation graph, the duals prove the board infeasible, and they localise the bottleneck.
  - **Better corridors.** The output is a *fractional* solution. Use the support {T : λ_{n,T} > δ} as a multi-corridor per net instead of one corridor.
  - **Warm start.** Seed the lattice history costs with log-prices (see idea 6).
- **Rounding.** Randomised rounding (Raghavan–Thompson, Combinatorica 1987) gives additive overflow O(log|R|/log log|R|) when capacities are small. PCB tile capacities are about 5–40 tracks per boundary per layer, so expect a violation of a few tracks, which the existing negotiation repairs.
- **Risk.**
  - Guarantees vanish at the fine lattice (capacity 1). Use this only at the coarse level.
  - The tile capacity model ignores intra-tile blockage, so capacity estimation matters a lot.
- **First experiment.** Replace the 16×16 corridor with the fractional support at ω = 0.1. Compare completion, iterations and wall-clock on the 4-layer boards.

### 5. Column generation / price-and-branch for exact global plans and bounds (V4, E3)
- **Refs.**
  - Wu, Davoodi & Linderoth, "GRIP: global routing via integer programming", TCAD 30(1) 2011 (also DAC'09, "GRIP: scalable 3D global routing using integer programming").
  - Carden, Li & Cheng, "A global router with a theoretical bound on the optimal solution", TCAD 1996 **(?)**.
  - Jain, Mahdian & Salavatipour, "Packing Steiner trees", SODA'03.
- **Mapping.**
  - Master LP: min Σ cost(T)·λ_{n,T} subject to capacities (idea 4's resources). Columns are trees.
  - Pricing: minimum-reduced-cost Steiner tree on the tile graph with dual edge costs. Exact for small nets (idea 16), heuristic for large ones. The Lagrangian bound stays valid if the pricing is exact.
  - Integer stage: price-and-branch over the column pool with HiGHS or CP-SAT.
  - PCB scale is small (≤400 nets, ~10³–10⁴ tile-edges), so the LP solves in seconds.
- **Gives:**
  - an optimality gap on wirelength plus vias;
  - duals for idea 18;
  - a clean place for "which net yields": use the **throughput variant** max Σ w_n z_n with Σ_T λ_{n,T} = z_n. The LP decides which low-priority nets to drop, globally.
- **Complements idea 4.** Idea 4 is fast and parallel; idea 5 is exact and interpretable. Pick one first; they share the oracle.
- **Risk.** An integrality gap at pinch points. Global plans can be detail-infeasible where tile capacity is optimistic.
- **First experiment.** Solve the LP on stalled boards and compare the LP bound and the rounded IP against PathFinder's corridors.

### 6. PathFinder as Lagrangian subgradient and congestion-game dynamics (V3, E1–2)
- **Refs.**
  - McMurchie & Ebeling, FPGA'95 (PathFinder).
  - Rosenthal 1973 (potential games).
  - Fabrikant, Papadimitriou & Talwar, STOC'04 (PLS-completeness).
  - Chien & Sinclair, SODA'07 (fast ε-Nash convergence).
  - Roughgarden, "Intrinsic robustness of the price of anarchy", STOC'09 / JACM'15.
  - Barahona & Anbil, "The volume algorithm", Math Prog 2000.
  - Fleischer, Jain & Mahdian, FOCS'04 (tolls).
- **Interpretation.**
  - History cost h_r += α(u_r − c_r)₊ is a subgradient step on the capacity multipliers.
  - The present-cost factor is a penalty or augmented-Lagrangian term.
  - With present cost only, and unweighted nets using identical resources, the game has Rosenthal's potential, so best response terminates (possibly after exponentially many steps).
  - History and growing present factors break the potential, so there is no convergence guarantee. That explains the stalls: they are cycles.
- **Concrete upgrades:**
  - (a) Polyak step size for history, t = (UB − Φ)/‖g‖², using Φ from idea 1.
  - (b) Volume-algorithm averaging. Keep per-net frequency-weighted routes as an approximate fractional primal. This shows which nets oscillate between which alternatives.
  - (c) Toll warm start. Fleischer–Jain–Mahdian show tolls exist that make the optimal flow an equilibrium (non-atomic case). LP duals from ideas 4/5 approximate them. Initialise h from them.
  - (d) Cycle detection. Hash the (net → route) state; a repeat means the dynamics are cycling. Hand off to idea 7 or 9 instead of iterating.
- **Guarantees.** Price-of-anarchy results bound *cost* in smooth games; they do not address feasibility. Treat this as diagnostics and step-size discipline only.
- **First experiment.** (c) plus (d) are about 100 lines. Measure iterations to legality.

---

## C. Exact local repair and bottleneck arbitration

### 7. CP-SAT / MaxSAT window LNS with UNSAT cores (V5, E3)
- **Refs.**
  - OptRouter: Kahng et al. (UCSD), ILP-optimal switchbox routing with complex rules ("Evaluation of BEOL design rule impacts using an optimal ILP-based detailed router", DAC'15).
  - Nam, Sakallah & Rutenbar, TCAD 2002 (SAT FPGA detailed routing).
  - Ohrimenko, Stuckey & Codish, Constraints 2009 (lazy clause generation).
  - Shaw, CP'98 (LNS).
  - Bayless et al., MonoSAT, AAAI'15.
- **Mapping.**
  - Window W: the bounding box of an overflow cluster, about 30×30 nodes × L layers, with k ≈ 5–25 nets.
  - Variables: x_{n,v} ∈ {0,1} for occupancy; arc variables for a unit flow per 2-terminal piece. Multi-pin nets use a multicommodity flow from the root.
  - Constraints:
    - Σ_n x_{n,v} ≤ 1;
    - net-class clearance as pairwise clauses x_{n,u} + x_{m,v} ≤ 1 for u, v within clearance;
    - via arcs forbid via footprints on all layers.
  - Boundary: the crossing points of each net are either fixed from the current solution or free within ±r nodes.
  - Objective: wirelength + β·vias. Alternatively MaxSAT: a soft literal "net n goes through W" weighted by its best detour cost outside W (A* around W).
  - Solve with assumptions; `SufficientAssumptionsForInfeasibility` returns a minimal conflicting set of nets.
- **Gives:**
  - an exact local answer: either a routing, or "these 4 nets cannot coexist here given the outside";
  - **the principled "which net yields" rule** (MaxSAT with detour weights);
  - an escape from integral stalls that idea 1 diagnoses.
  - With MonoSAT, reachability is a native theory, so no flow encoding is needed.
- **Guarantees.** Exact within the window model. UNSAT is conditional on the boundary, so relax boundaries or grow W before declaring infeasibility.
- **Risk.**
  - Encoding size with fine clearances.
  - False UNSAT from tight boundaries.
  - CP-SAT solve time can vary widely; use short time limits.
- **First experiment.** On a stalled 2-layer board, extract the three worst windows and give CP-SAT 10 s per window. Measure resolved overflow and core sizes.

### 8. Bottleneck arbitration by auction and opportunity cost (V4, E2)
- **Refs.** Bertsekas, "The auction algorithm", Annals of OR 1988. VCG allocation (unit demand).
- **Mapping.**
  - Passage b: the resources R_b with capacity c_b, claimed by N_b > c_b nets.
  - For each net n: δ_n = cost(best route avoiding R_b) − cost(best route through R_b), at current prices.
  - Give the c_b slots to the nets with the largest δ_n. This is VCG-efficient for unit demand.
- **Extensions.**
  - Ordered slots: track positions across a channel must respect planar order to avoid crossings. The allocation becomes a non-crossing b-matching, an O(k²) interval DP.
  - Several interacting bottlenecks: run Bertsekas's auction on (nets × slots). Its ε-complementary-slackness prices become history costs.
- **Replaces** "rip up most-contested nets", which ignores who is cheapest to detour.
- **Risk.** δ_n is myopic, since detours create new conflicts. Mitigate with a couple of auction rounds or by combining with idea 9.
- **First experiment.** At stall time, compute δ for the top-3 bottlenecks, force the allocation, and continue negotiating.

### 9. Ejection chains / Lin–Kernighan-style rip-up (V4, E2)
- **Refs.**
  - Glover, "Ejection chains…", Discrete Applied Math 1996.
  - Ahuja, Ergun, Orlin & Punnen, "A survey of very large-scale neighborhood search", DAM 2002.
- **Mapping.**
  - Ejection graph: nodes = nets; arc A→B weighted by A's gain from taking B's resources plus B's detour cost.
  - Look for a negative-cost path or cycle up to depth d, with a tabu on moved nets.
  - Accept the best prefix, LK-style.
- **Complements** the "reinsert against hard obstacles" fallback, which is a depth-1 chain.
- **Risk.** Evaluating arcs needs A* calls. Restrict to nets that share resources with the blocked net.
- **First experiment.** Depth-3 chains at the stall point; count completions gained per second.

---

## D. Copper pours (pain 1)

### 10. Island-aware routing costs via planar duality (V5, E2)
- **Refs.**
  - Tree–cotree duality (Whitney; a spanning tree's complement in the dual is a spanning tree).
  - Holm, de Lichtenberg & Thorup, JACM 2001 (fully dynamic connectivity).
- **Mapping.**
  - Per layer ℓ, build the obstacle graph G_ℓ:
    - one node per other-net copper object inflated by pour clearance plus half the minimum neck width;
    - one node for the board outline or keepout;
    - an edge when two inflated obstacles touch.
  - The pour splits into roughly 1 + β₁(G_ℓ) pieces, where β₁ = |E| − |V| + #components. Each independent cycle encloses a face, a potential island.
  - A new track segment creates an island **iff it touches two obstacles already in the same union-find component**. Every track starts at a pad, which is itself an obstacle, so the test is local.
- **A* integration.**
  - Carry in the label a small bitset (≤4) of the components the partial path has touched. Charge λ_island when a step touches a component already in the set.
  - Waive the penalty if the enclosed face contains a pour-net pad, or has room for a stitching via onto connected pour on the other layer.
  - Rebuild the union-find each negotiation iteration (near-linear), or keep it dynamic (Holm et al.) under rip-up.
- **Relation to the "plane skeleton".** The skeleton reserves a primal spanning tree of pour cells. Island cost forbids dual cycles. Tree–cotree duality says these are the same constraint seen from both sides, but island cost is *soft and local*: the router can shred where it is harmless.
- **Risk.** A heuristic label (non-exact because of the component-set truncation). Face-area and contains-pad tests need geometry.
- **First experiment.** Add the penalty on the 85–90% boards. Measure the island count after pour refill, and completion.

### 11. Pour healing as a Steiner / prize-collecting / 2-connected design problem, with cut feedback (V4, E2)
- **Refs.**
  - Ljubić et al., "An algorithmic framework for the exact solution of the prize-collecting Steiner tree problem", Math Prog 2006.
  - Goemans & Williamson 1995 (PCST 2-approximation).
  - Jain, Combinatorica 2001 (iterative rounding for survivable network design).
- **Mapping.**
  - Island graph: nodes = pour islands on each layer.
  - Candidate edges: a stitching via where island i on L1 overlaps island j on L2 and a via fits; or a short bridge track. Cost = 1 via, or a length.
  - Terminals: islands containing pour-net pads. SMD pads connect only their own layer; THT pads join the islands on all their layers.
  - Minimum Steiner tree = fewest stitches. PCST with prize = island area decides which dead islands to keep or remove. Requiring 2-edge-connectivity (≥2 vias per link) gives current and EMI robustness.
  - Islands number in the hundreds, so an exact ILP (directed cut, branch-and-cut) is instant.
- **Feedback.** If terminals are disconnected in the candidate graph, the minimum cut (blocked adjacencies) names the signal segments that separate ground islands. Raise the pour-resource price on exactly those segments and reroute. That is a combinatorial Benders loop between signal routing and pour connectivity.
- **Replaces** the ad hoc "stubs/vias/stitching" rung of the fallback ladder.
- **First experiment.** Run the Steiner ILP post-route on the failing boards and compare via counts and residual disconnections against the current stitcher.

### 12. Exact layer assignment by planar max-cut (2D-projection-then-layers) (V3, E3)
- **Refs.**
  - Chen, Kajitani & Chan, TCAS 1983 (constrained via minimisation, optimal for junction degree ≤ 3).
  - Pinter 1984 **(?)**.
  - Hadlock, SIAM J. Comput. 1975 (planar max-cut in polynomial time).
  - Barahona, Grötschel, Jünger & Reinelt, Operations Research 1988.
  - Grötschel, Jünger & Reinelt, ZAMM 1989 (via minimisation with pin preassignments and layer preference).
- **Mapping.** Fix the 2-D projection of the routes. Segments that cross must lie on different layers. For 2 layers, minimum vias is max-cut on a planar cluster graph, solvable exactly in polynomial time.
- **Add a pour objective.** Unary costs "segment on layer ℓ damages that layer's pour" (for example, area or island creation per idea 10). Unary terms are apex edges and break planarity. The result is general max-cut / QUBO, which a few thousand segments make ILP-tractable with branch-and-cut.
- **Payoff.** You can designate a "mostly pour" layer and push signals off it optimally.
- **Also.** Run the router as 2.5-D (projection with crossing costs), then assign layers exactly.
- **Risk.** Fixing the projection gives up freedom; unconstrained via minimisation is much harder.
- **First experiment.** Post-process routed 2-layer boards and compare via count and pour connectivity.

---

## E. Escape routing (pain 2)

### 13. BGA escape by exact channel capacities, flow, and SAT modulo graphs (V4, E3)
- **Refs.**
  - Yan & Wong, "A correct network flow model for escape routing", DAC'09. Correct diagonal-capacity model; optimal single-layer escape.
  - Bayless, Hoos & Hu, "Scalable, high-quality, SAT-based multi-layer escape routing", ICCAD'16 (MonoSAT).
  - Luo & Wong, "Ordered escape routing based on Boolean satisfiability", ASP-DAC'08.
  - Leiserson & Pinter, "Optimal placement for river routing", SIAM J. Comput. 1983.
- **Mapping.**
  - Channel graph instead of the lattice. The orthogonal channel between balls holds k = ⌊(p − d − c)/(w + c)⌋ tracks; the diagonal channel uses p√2 − d. Computed exactly, so the 0.01 mm slack is not lost to discretisation.
  - Single-layer unordered escape is max-flow with node capacities, exact.
  - The min-cut is a Hall-type certificate: "these m pins sit inside a boundary of capacity < m".
  - Multi-layer escape (dog-bone vias, layer choice per pin) with design rules: MonoSAT flow and reachability constraints, per Bayless et al.
  - Ordered escape, matching the fan-out order to the destination to avoid later crossings: Luo–Wong SAT. For connector-to-BGA buses: river routing.
  - Geometry: evenly space the k tracks inside each channel, or use the LP of idea 3.
- **Coupling to global routing.** Choose each pin's escape boundary slot by min-cost flow, with slot costs from the global duals (idea 4).
- **Risk.** Hand-over between escape and global routing. MonoSAT is C++; it would need FFI or an equivalent re-encoding in CP-SAT.
- **First experiment.** Max-flow escape on the failing BGA with exact capacities; check whether the flow value reaches the pin count.

---

## F. Missing features (pain 6)

### 14. Pin and gate swap: super-sink in negotiation plus linear assignment (V4, E1–2)
- **Refs.**
  - Betz & Rose, VPR, FPL'97 (logically equivalent pins via a shared sink).
  - Eades & Wormald, Algorithmica 1994 (bipartite crossing minimisation; median heuristic, 3-approximation).
  - Kuhn's Hungarian method.
- **Mapping.**
  - Pin swap: an equivalence group becomes a virtual super-sink with an arc to each member pin, each pin with capacity 1. A* targets the super-sink, and PathFinder negotiates the assignment for free. This is exactly VPR's mechanism.
  - Gate swap (bundles of pins that must move together): logical-to-physical gate assignment is a linear assignment problem, costs = sum of estimated route costs at current prices. k ≤ 8, so Hungarian is instant; re-solve each outer iteration.
  - Decoupling caps on the same VCC/GND pair: the cap-to-power-pin assignment is also a linear assignment.
  - Fully swappable ordered buses: an order-preserving assignment gives zero crossings (river routing), which matters most on 2-layer pour boards.
- **Risk.** Back-annotation to the schematic; swap-group metadata from KiCad.
- **First experiment.** The super-sink on connector pins marked swappable.

### 15. Differential pairs and length matching: product graph, RCSP and area transportation (V3, E3)
- **Refs.**
  - Ozdal & Wong, "A length-matching routing algorithm for high-performance PCBs", TCAD 25(12) 2006.
  - "LP-based area assignment for length-matching routing of complex multilayer PCBs with any-direction wires", TODAES 2026. Recent; I did not verify the authors.
  - Handler & Zang, Networks 1980 (constrained shortest path).
  - Irnich & Desaulniers 2005 (resource-constrained shortest path chapter).
- **Mapping.**
  - Diff pair: one commodity on a state-expanded lattice (node, direction). Footprint = two tracks plus the gap. Turn states carry inner/outer length compensation. A bounded uncoupled breakout segment at each pad.
  - Maximum-length bound: resource-constrained shortest path, via Lagrangian relaxation of length plus labelling.
  - Minimum-length bound: route first, then meanders.
  - Meander feasibility is a **transportation / max-flow problem**: free regions r supply area A_r; net n demands Δ_n × (meander pitch); arcs connect adjacent pairs.
  - Infeasibility is a Hall violation, a readable certificate: "these 3 nets need 14 mm extra, adjacent free area supports 9 mm".
- **Risk.** The product graph multiplies the state space; coupled-gap geometry on a 45° lattice is fiddly.
- **First experiment.** The area-transportation check on existing routes of length-matched groups.

---

## G. Steiner trees

### 16. Exact goal-oriented Steiner trees for small nets; better heuristics for large ones (V3, E2)
- **Refs.**
  - Hougardy, Silvanus & Vygen, "Dijkstra meets Steiner: a fast exact goal-oriented Steiner tree algorithm", MPC 2017.
  - Takahashi & Matsuyama 1980.
  - Mehlhorn, IPL 1988 (Voronoi-based 2-approximation).
- **Mapping.**
  - The current tree growing is Takahashi–Matsuyama: a 2(1 − 1/k)-approximation, order-dependent.
  - For k ≤ 6–8 terminals, use exact Dijkstra–Steiner inside the corridor. Its future-cost lower bounds are A* heuristics, so they fit the existing A* stack.
  - For pour or power nets with hundreds of pins, use Mehlhorn plus key-path exchange local search.
  - The same exact solver provides the valid lower bounds needed by ideas 1 and 5.
- **Risk.** 3^k blow-up; keep a cap on k.
- **First experiment.** Swap in exact trees for nets with 3–6 pins; measure wirelength and overflow.

---

## H. Placement (pains 3, 4)

### 17. CP-SAT NoOverlap2D legalisation for dense two-sided boards, with packing bounds (V5, E2)
- **Refs.**
  - Beldiceanu & Carlsson, CP'01 (sweep pruning for non-overlapping rectangles; CP-SAT's NoOverlap2D propagator uses energetic reasoning).
  - Murata et al., ICCAD'95 (sequence pair).
  - Fekete & Schepers, Math. Methods of OR 2004 (dual-feasible-function bounds for orthogonal packing) **(?)**.
  - Brenner & Vygen, TCAD 2004 (flow-based minimum-movement legalisation). Row-based, so it is the analogue for the fixed-order LP.
- **Mapping.**
  - Per component: side literal s_i; orientation class (0/180 vs 90/270, same bounding box); integer x_i, y_i on a grid of about 0.05 mm.
  - Optional interval pairs per (side, orientation class) with a shared presence literal, and one NoOverlap2D per side.
  - THT parts: present on both sides.
  - Non-rectangular courtyards: a union of boxes sharing offsets.
  - Fixed and forced-side constraints for tall or THT parts.
  - Objective: Σ|x_i − x̂_i| + |y_i − ŷ_i| + flip penalty. Hints from SA or ePlace.
  - With the pairwise relative order fixed (sequence pair), minimum displacement is an LP of difference constraints whose dual is min-cost flow. Use it for fast polish.
- **Infeasibility.** CP-SAT proofs of UNSAT for tight packings can be slow. Add cheap necessary conditions first:
  - per-side area including keepouts;
  - forced-side subsets;
  - conflict cliques of "wider than W/2" items;
  - dual-feasible functions.
- **Replaces** the SA legaliser when SA fails; keep SA for the easy cases.
- **Risk.** Grid resolution vs slack; arbitrary-angle parts.
- **First experiment.** Run CP-SAT for 60 s on the boards where the placer finds nothing legal.

### 18. Routability coupling via duals and combinatorial Benders cuts (V4, E3)
- **Refs.**
  - Hooker & Ottosson, Math Prog 2003 (logic-based Benders decomposition).
  - Codato & Fischetti, Operations Research 2006 (combinatorial Benders cuts).
- **Mapping.**
  - (a) Lagrangian coupling: add Σ_n Σ_e y_e·P_ne(x) to the ePlace objective. y_e are the global-routing duals (ideas 4/5); P_ne is a smooth, RUDY-style probability that net n crosses tile edge e given its pin positions.
  - (b) Cut cuts: for each violated geometric cut Γ (idea 2), add the penalty μ·max(0, D_Γ(x) − κ_Γ(x)).
    - D_Γ is a sigmoid-smoothed count of nets straddling Γ; for multi-pin nets, 1 − ∏(same side).
    - κ_Γ grows as the obstacles bounding the gap move apart.
    - Anchor cuts to component pairs (for example, "the gap between U3 and U7") so they move with the parts.
  - Cuts accumulate over outer loops, Benders-style.
- **Replaces** the "route, nudge, reroute" loop as the primary coupling; the router becomes a cut generator instead of an expensive oracle.
- **Risk.** Cuts are placement-local; smoothing makes the penalty landscape noisy.
- **First experiment.** Feed idea 2's violated cuts back as penalties for 3 outer loops; measure routed completion.

### 19. Ratsnest crossing minimisation for 2-layer boards with pours (V4, E2)
- **Refs.** Crossing-minimisation literature (Eades–Wormald). I found no direct PCB paper, so treat this as a hypothesis.
- **Reasoning.** On 2 layers with pour on both, every pair of connections that must cross forces one of them onto the other layer locally, which cuts that layer's pour.
  - With pins on the boundary of a region, crossings are forced by interleaving (chord-diagram crossings), so the count is a lower bound there.
  - In general, crossings can be traded for detours at a wirelength cost.
- **Mapping.**
  - Add a smooth crossing count of MST-ratsnest segments (sigmoid of orientation determinants) to the analytic objective.
  - Use an exact count via a sweep in SA and refinement, including quarter-turn and swap moves.
  - Combine with pin swap (idea 14).
- **Risk.** A proxy only; the weight needs tuning against wirelength.
- **First experiment.** Correlate ratsnest crossings with final completion and island count on the existing corpus. It needs no code beyond a counter.

### 20. Very-large-scale neighbourhoods: exact permutation of identical footprints (V3, E1)
- **Refs.**
  - Pan, Viswanathan & Chu, FastDP, ICCAD'05 (independent-set matching).
  - Ahuja, Ergun, Orlin & Punnen, DAM 2002 (VLSN survey).
- **Mapping.**
  - Components with identical courtyards (0402 caps, resistors) can be permuted among their current sites without breaking legality.
  - Cost c_ij = HPWL of i's nets with i at site j and the best orientation. This is exact when no net joins two members of the group; otherwise iterate, because the true problem is a QAP.
  - Hungarian in O(k³).
  - Cyclic-exchange neighbourhoods for non-identical parts that fit each other's sites: detect negative cycles in the improvement graph.
- **First experiment.** Add as a refinement pass; measure HPWL and routing completion.

---

## I. Parallelism and speed (pain 5)

### 21. Nested dissection of routing, deterministic batches, and admissible landmarks (V4, E2–3)
- **Refs.**
  - Gort & Anderson, FPT'10 (deterministic parallel FPGA routing, about 2.3× on 4 cores).
  - GRIP's subregion decomposition (above).
  - Lipton & Tarjan 1979 (planar separators).
  - Goldberg & Harrelson, SODA'05 (ALT landmarks).
- **Mapping.**
  - (a) Separator decomposition. Take the separator crossings of long nets from the global plan (idea 4), fix them as pseudo-pins, then route the two halves independently and recursively in parallel. Renegotiate only near separators.
  - (b) Deterministic batches. Colour the conflict graph of net corridors (greedily, or interval colouring of bounding boxes). Nets in one colour class are routed in parallel, which is exactly equivalent to sequential routing.
    - Jacobi-style simultaneous best response oscillates. Damping or MWU step sizes fix this, which is what the idea 4 theory covers.
  - (c) **ALT landmarks stay admissible across all PathFinder iterations**, because present and history factors never drop below the base cost. Distances under the base costs are therefore a permanent lower bound. Compute landmarks once per board and layer, for large cuts in A* node expansions.
- **Risk.** (a) Fixed separator crossings can be suboptimal; allow ±r slack as GRIP does.
- **First experiment.** (c) is quick to try; then (b).

---

## J. Niche

### 22. Frontier DP / ZDD over narrow strips (V2, E3)
- **Ref.** Kawahara et al., frontier-based search (IEICE 2017) **(?)**.
- **Mapping.** Routing disjoint paths in a strip of width w (for example, between BGA rows or in connector fields) is a DP over non-crossing frontier partitions, O(n·Catalan(w)). For w ≤ 10–12 it can enumerate *all* routings, count them, or prove there are none.
- **Assessment.** Mostly subsumed by idea 7; worth it only if strip-shaped bottlenecks dominate.

---

## Top 5

1. **Resource-sharing (MWU) global router on a geometric-capacity tile graph, with dual warm start (ideas 4 + 6c + 21a).**
   - It is the one change that addresses pains 4, 5 and 7 together: a fractional multi-corridor, parallel with stale prices, λ* and duals as a certificate and bottleneck map, and duals as tolls that seed history.
   - Proven in BonnRoute at far larger scale.
   - Guarantees survive at the coarse level, where capacities are 5–40, and vanish only at the lattice level, where negotiation stays.
2. **The certificate stack (ideas 1 + 2 + 13's min-cut + 17's bounds).**
   - Idea 1 costs almost nothing, since you already compute the shortest paths and prices, and it immediately separates "lattice-infeasible" from "gave up".
   - Idea 2 gives human-readable geometric proofs and feeds the placer (idea 18).
   - Together they are pain 7's answer, with the level of each claim (lattice vs geometry vs window) stated honestly.
3. **Pour handling by duality: island-aware A* costs (10) plus Steiner/PCST pour healing with cut feedback (11).**
   - Directly targets the 85–90% stall.
   - Theory (tree–cotree duality, Steiner ILP) turns "pour stays connected" from a fallback ladder into a priced, local constraint and an exactly solved repair with a Benders loop.
   - Cheap: union-find plus an ILP with a few hundred nodes.
4. **CP-SAT / MaxSAT window LNS with UNSAT cores (7), with ejection chains (9) as the cheap tier.**
   - Resolves the integral stalls that negotiation cycles on.
   - Gives principled "which net yields" (MaxSAT detour weights).
   - Produces local infeasibility proofs; also usable for BGA pockets.
5. **CP-SAT NoOverlap2D legalisation with side and orientation literals (17).**
   - Pain 3 is a pure packing-feasibility failure, where SA is the wrong tool. CP-SAT finds placements SA misses, and cheap packing bounds certify impossibility.
   - Low effort: a few hundred lines against OR-Tools' existing propagators.

**Close runners-up.** Promote these if the board mix warrants:
- **13 (BGA channel flow + LP realisation, ideas 13 + 3)** if fine-pitch BGAs are frequent. It is the only fix for 0.01 mm slack that does not fight the lattice.
- **14 (super-sink pin swap)**: best value per line of code.
- **21c (ALT landmarks)**: likely the quickest wall-clock win on 4-layer boards.
