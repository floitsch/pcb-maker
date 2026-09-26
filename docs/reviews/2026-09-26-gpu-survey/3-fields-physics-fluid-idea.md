# Fields, PDEs and continuum methods for a GPU PCB router: evaluation of the fluid/colour idea

**Bottom line.** A literal fluid is the wrong model: traces are thin 1-D objects of fixed width, and immiscible fluids minimise interface area, so they make blobs, not wires. But each part of the owner's idea maps onto a known continuum object that runs well on a GPU:

- **"Distance and length propagated through the field"** is the eikonal arrival-time field.
- **"Colour pushes toward its counterpart"** is back-tracing down the counterpart's arrival-time field.
- **"Strengthen or lock finished connections"** is Physarum / dynamic Monge–Kantorovich conductance growth, where tubes thicken with flux. This growth is also what turns spreading diffusion into shortest paths and trees.
- **"Colours competing for space"** is a multi-label territory field or a congested-transport (Wardrop) equilibrium. Continuum theory shows that PathFinder's prices are the right dual variables, so the field methods refine PathFinder rather than replace it.

What I'd bet on: **a GPU eikonal PathFinder built on min-plus parallel scans (GAMER-style), with clearance taken from exact signed-distance fields, damped Jacobi or coloured Gauss–Seidel negotiation across nets, coarse-to-fine corridors, and a population of randomised configurations.** Physarum dynamics, colour territories and a resistor model of the pour are the best add-ons from the field family.

---

## 0. Why the prior diffusion experiment gave detours, and the one-line fix

- A Laplace/Darcy potential minimises ∫c|∇u|² (an L² energy), so current spreads over many parallel paths. Steepest descent follows current streamlines, and streamlines are not geodesics. That explains 42 vs 28 cells.
- Shortest paths are the **L¹ (Beckmann) problem**: min ∫c|F| subject to div F = f. There are three known ways to get from L² to L¹:
  1. **Screening (leak to ground).** Solve −∇·(k∇u) + u/ε² = 0 with u = 1 at the source. Then −ε·log u tends to the geodesic distance as ε→0 (Varadhan's formula; this is the basis of the heat method). Adding a leak conductance at every node and descending −log u instead of u should roughly recover 28 and 21. The catch is fp32 underflow of exp(−d/ε). Working in the log domain turns the linear solve into a nonlinear soft-min Bellman–Ford, which is eikonal again.
  2. **p-Laplacian with p→∞.** The potential tends to the distance (Kantorovich potential) and the flux concentrates on transport rays (Evans–Gangbo 1999). This is a useful theoretical link but badly conditioned in practice.
  3. **Conductance adaptation (Physarum / DMK).** μ̇ = |μ∇u|^β − μ. For β = 1 it provably converges to the shortest path on graphs (Bonifaci–Mehlhorn–Varma 2012). β > 1 concentrates flow into branched trees (Facca–Cardin–Putti 2021).

So the earlier conclusion ("fields are only good for prices") holds for static-conductance diffusion. It does not hold for adaptive-conductance or eikonal fields.

---

## 1. Ideas, the owner's idea and its best variants first

Common notation below: **h** = fine cell size, **L** = number of layers (≤ 4), **N** = number of nets, **bbox** = a net's bounding window.

### A. The fluid/colour idea and its variants

**1. Literal immiscible multi-component fluid (Shan–Chen lattice Boltzmann or Navier–Stokes, one colour per net).**
- Memory: about 9 distributions per cell per component, times N. Impossible at N = 200, and poor even at 10.
- How immiscibility works: surface tension, which minimises interface length. That makes the physics actively harmful: thin filaments retract and pinch off, and colours form round droplets.
- Inertia only adds overshoot. Mass conservation means colours fill area rather than forming wires.
- Width control would come only from the diffuse-interface thickness, and vias have no natural analogue.
- **Verdict: kill.** It is shown only to explain why the analogy breaks.

**2. Per-net resistor field with screening and log-potential descent (the prior experiment, repaired).**
- One scalar per net per bbox. Each net needs one linear solve per iteration: multigrid is about 10–20 memory-bound passes.
- Descending −log u gives near-geodesic paths. Pin trees can be built with the pin-to-tree Prim heuristic (the tree becomes the Dirichlet source).
- Clearance and immiscibility come only through prices, exactly as in PathFinder.
- **Verdict:** a 1-hour sanity experiment that confirms the diagnosis. Dominated by eikonal sweeps: same answer, no linear solve, no underflow.

**3. Physarum / dynamic Monge–Kantorovich per net: the "strengthening" mechanism.**
- Per net, in its bbox: potential u_n and conductance μ_n.
- Each step: solve ∇·(μ_n∇u_n) = f_n. Then update μ_n ← μ_n + dt(|μ_n∇u_n|^β − μ_n + μ_min).
- **Multi-pin trees:**
  - Option A: one pin is the sink and the others share the source.
  - Option B: rotate which pin is the source/sink pair each step (Tero et al.'s rule, as used in the 2010 Tokyo-rail paper).
  - β ∈ [1, 1.5] moves from straight connections toward Steiner-like trees (branched transport).
- **"Lock finished":** μ saturating at μ_max is a natural hysteresis, since a thick tube carries all flux and starves alternatives.
  - Freeze a net once max|Δμ|/μ stays below a tolerance and the net is conflict-free for K steps.
  - A frozen net is rasterised into the obstacle field.
- **Vias:** vertical conductance μ_z at each (x,y), with its own adaptation and a baseline scaled by 1/c_via. A via appears where μ_z is large. Via legality is a mask (see 21).
- **Immiscibility:** add competition between nets, μ̇_n ∝ … − γ·μ_n·D_r[Σ_{m≠n} μ_m], where D_r dilates by w_n/2 + clr + w_m/2. This is Lotka–Volterra competition between tubes; see 4.
- **Extraction:** threshold μ_n, skeletonise, then string-pull (see 22).
- **Cost:** convergence takes hundreds to about a thousand steps, each with a Poisson solve per net. On a coarse grid this is about 5× the cost of an eikonal PathFinder run (estimate in §3).
- **Diversity:** random initial μ, a random source-rotation schedule, random β.
- **Risks:** slow; integrality only emerges asymptotically; loops can persist (Physarum is known to converge to the triangle's perimeter rather than a tree).
- **Verdict: top 5, coarse grid only.** It is the principled answer to "strengthen and lock" and to multi-pin topology.

**4. Multi-commodity Physarum with immiscible colours.**
- The owner's idea has literally been published:
  - Lonardi–Facca–Putti–De Bacco (Phys. Rev. Research 2021): multi-commodity optimal-transport dynamics.
  - Lonardi–Baptista–De Bacco (Front. Phys. 2023), "Immiscible color flows in optimal transport networks": commodities share edge capacity.
- In that work "immiscible" means per-colour fluxes sharing a capacity-limited conductance. Colours are **miscible in space**, penalised only through shared capacity. For PCB routing, sharing must become *exclusion with clearance*. So:
  - replace the shared conductance with per-net μ_n,
  - add the dilated competition term above,
  - add a price field p(x) (the dual of the capacity constraint) that feeds back into all conductances.
- Memory is 2 fields per net per bbox. That is fine coarse, too much fine.
- **Verdict:** the right research formulation of "colours pushing toward counterparts". Use it at 0.4–0.5 mm to produce topology and corridors, not final routes.

**5. Colours as arrival-time wavefronts (eikonal per net).**
- The arrival-time field T_n(x) = geodesic distance from net n's current tree under the price metric c(x) = (base + history)·present. This is exactly "distance propagated through the field".
- "Pushing toward the counterpart" is steepest descent on T_n from the target pin (back-trace).
- **Multi-pin in one pass:** seed all pins at once with a "which pin reached here first" label. This gives the geodesic Voronoi diagram of the pins. For every Voronoi-boundary edge (a,b), the connection cost is T_a + c + T_b. Taking the MST over those boundary edges is Mehlhorn's (1988) 2-approximation to the Steiner tree. That is one wave per net per iteration instead of k−1.
- **Vias:** relaxation along the layer axis, with legality and cost from a column field (21).
- Memory: one u32 per node per net in bbox (distance and parent direction packed; see §3).
- **Verdict: the core of the recommended design** (see 9 for the kernel).

**6. Shared-field colour territories (geodesic-Voronoi competition, auction dynamics).**
- This solves the memory problem: store **one label and one distance per cell for all nets together**.
- Every pin floods with speed 1/(c(x)·w_n) and each cell takes the label of the first arrival. Each net has a scalar "pressure" (weight or bid) π_n.
- A net whose pins lie in more than one connected component of its own territory (found by GPU connected-component labelling) raises its bid; nets with slack lower theirs. This is auction dynamics (Jacobs–Merkurjev–Esedoglu 2018), with "connectivity of pins" playing the role of volume constraints.
- **Clearance:** route each net's centreline inside its territory at distance ≥ w_n/2 + clr/2 from the territory boundary. Each side keeps half the clearance, so any two territories' traces are legal automatically. Territory width then gives capacity directly.
- **Vias:** per-layer territories, plus a via-legal mask requiring the same label or free space in all L layers within r_via.
- **Pour:** the pour is the *background label*: whatever nobody claims.
- **Lock:** a net's label becomes fixed (it stops bidding) once its territory is connected and its route is feasible.
- **Kernel:** jump flooding (Rong–Tan 2006) or sweeps for the multi-label flood, plus connected-component labelling. Several ms per round at full resolution.
- **Diversity:** random initial bids and seeding jitter give very cheap, very different topologies.
- **Risks:** territory partitions waste area (Voronoi cells are fat); packing many nets through one channel requires thin territories, which pure first-arrival competition does not produce. Fix with capacity-aware speeds.
- **Verdict: top 5,** as a global coordinator and initial-price generator.

**7. Multi-phase field (Allen–Cahn / MBO threshold dynamics) with sparse top-K phases.**
- The grain-growth phase-field trick: store only the K ≈ 3–4 largest phase values and their labels per cell, so memory is independent of N. (This is standard in large grain-growth phase field; I did not verify a specific citation.)
- MBO step: diffuse (a short heat step), then take the argmax. The multi-phase version with arbitrary surface tensions is Esedoglu–Otto (CPAM 2015).
- It gives smooth territory boundaries, but curvature flow shrinks thin necks and does not preserve pin connectivity.
- **Verdict:** a regulariser for 6. On its own it would produce blobs, like idea 1.

### B. Shortest paths as PDEs

**8. Fast sweeping / fast marching / fast iterative method in general.**
- Fast sweeping (Zhao 2005) runs Gauss–Seidel sweeps in alternating orders; 2^d sweeps suffice for a pure distance field.
- The fast iterative method (FIM; Jeong–Whitaker 2008) keeps an active list and works well on a GPU.
- Near-Far (Davidson et al. 2014) is the work-efficient graph variant.
- Fast marching is serial (heap-based); skip it.

**9. Min-plus parallel-scan sweeping on the routing lattice (the workhorse).**
- A directional sweep along a row with additive costs is a scan over tropical affine maps, x ↦ min(x + a, b). The composition (a₁,b₁)∘(a₂,b₂) = (a₁+a₂, min(b₁+a₂, b₂)) is associative.
- So each row, column or diagonal is one workgroup scan: O(log n) depth, no subtraction, no precision loss.
- GAMER (Lin, Liu, Young, Wong; ICCAD 2021 and TCAD 2023) does exactly this for VLSI maze routing: alternating horizontal and vertical sweeps with parallel scans, a 16× speedup inside CUGR, no quality loss.
- For octilinear routing: 4 axis directions plus 4 diagonals (diagonals via skewed indexing), plus a per-thread vertical relax over L ≤ 4.
- Iterations: about 1 + (number of bends in the optimal path) rounds. Warm starting from the previous iteration's field cuts this sharply.
- **Verdict: top 5, number 1.**

**10. Active-list Bellman–Ford with atomicMin on packed u32.**
- For irregular corridors where rectangular scans waste work: pack (fixed-point distance << 3 | parent direction) into a u32 and relax with atomicMin. This is legal in WGSL: u32 atomics exist, float atomics do not.
- A good complement to 9 inside fine corridors.

**11. Heat method / screened Poisson (Crane–Weischedel–Wardetzky 2013).**
- Strong on meshes with a prefactored Laplacian. On regular grids with changing obstacles and inhomogeneous prices you re-solve two elliptic problems every iteration.
- It is smoothed (distance is accurate only at scale √t), and a Riemannian metric needs an anisotropic diffusion tensor.
- **Verdict:** loses to 9 on every axis here, except that it is differentiable, and 14 covers that better.

**12. Turn-penalised (lifted) eikonal on (x, y, θ) with 8 headings.**
- Mirebeau et al. (2023) compute globally optimal curvature-penalised paths (Reeds–Shepp, Dubins, Euler elastica) on a GPU.
- For PCBs, a discrete version with 8 headings, a 45° turn cost and a large 90° cost yields few-bend octilinear paths that extract cleanly to KiCad 45° geometry.
- 8× memory and work, so use it only in the fine stage, in corridors, for the top configurations.

**13. Bit-parallel Lee cellular automaton.**
- Unit-cost wave expansion is binary dilation of a bitmask: 32 cells per u32.
- With Akers' 2-bit wave coding (the 1,1,2,2 sequence) back-tracing works from 2 bits per cell. So *all 200 nets concurrently at full resolution*: 200 × 12 M × 2 bit ≈ 600 MB.
- Costs must be integer "delays", so congestion granularity is coarse, and the wave count equals path length in cells (hundreds to thousands of dilations).
- **Verdict:** a neat fallback when memory is the binding constraint, and good for fast "is it routable at all" probes. Not the main path.

### C. Transport and equilibrium (the theory behind negotiated congestion)

**14. Entropic (soft-min) congested transport: a differentiable PathFinder.**
- Replace min with −ε·logsumexp(−·/ε) in the sweeps (a log-domain scan; still associative).
- The gradient of the soft distance with respect to cell costs is the expected cell usage of net n: a smooth "colour density", the fluid the owner imagined.
- Sum the densities, set price = g(total density, capacity) (a Wardrop / Carlier–Jimenez–Santambrogio 2008 equilibrium), and run mirror descent or Frank–Wolfe on the prices while annealing ε→0.
- This is PathFinder with principled dual updates. History cost is dual ascent on the capacity multiplier.
- Cost: about 2× idea 9 (forward pass plus back-propagation).
- Uses: principled price updates, smooth congestion maps, and gradients the placer can use.
- **Verdict: top 5.**

**15. Beckmann fractional multicommodity flow as a routability oracle and lower bound.**
- The relaxed (fractional, capacitated) continuum problem is convex. If it is infeasible, the discrete problem is infeasible.
- The dual prices give a far better congestion map than RUDY-style estimates.
- Feed it to the placer to kill bad placements before routing. Analogous to electrostatic GPU placement (ePlace/DREAMPlace; not re-verified this session).

**16. Sinkhorn optimal transport (Cuturi 2013) for assignment subproblems only.**
- Pin and gate swapping, bus escape ordering, layer assignment of bundles, pad-to-fanout-via matching.
- Tiny matrices, GPU-trivial. Not a router by itself.

**17. Branched transport (Xia 2003) and Gilbert networks for multi-pin topology.**
- The α-cost (flow^α per length, α < 1) favours trunks.
- In practice this is DMK with β > 1 (idea 3), or a Gilbert-Steiner heuristic on the Voronoi graph from idea 5.
- Useful only for nets with many pins (power, clocks). A GND net that is poured does not need it.

### D. Growth and pattern formation

**18. Competitive dielectric breakdown / DLA growth (Niemeyer–Pietronero–Wiesmann 1984).**
- Each net grows from its pins with probability ∝ |∇φ|^η toward its counterparts; nets block each other as they grow.
- With large η this is randomised greedy steepest descent, and each growth step needs a Laplace solve.
- Replacing the Laplace field with an arrival-time field gives a cheap *parallel randomised greedy* initialiser for PathFinder.
- **Verdict:** a diversity generator, not a router.

**19. Reaction–diffusion / Turing patterns.**
- Gray–Scott labyrinths are densely packed channels at a fixed wavelength, which could be tuned to track pitch.
- No control over connectivity or endpoints. **Kill.** At most a curiosity for bus escape patterns.

**20. Topology optimisation (SIMP / density methods) per net.**
- Per-net density ρ_n; minimise resistance between pins plus a volume term plus an overlap penalty Σ ρ_n·D[ρ_m].
- The robust erode/intermediate/dilate projection (Wang–Lazarov–Sigmund 2011) enforces a minimum solid size *and* a minimum gap. That is exactly track width and clearance, which is its best feature.
- Essentially Physarum written as an optimisation, and expensive at N = 200.
- **Verdict:** use it where it is uniquely good: **shaping the pour** (widening necks, removing slivers and acid traps) and fattening power traces after routing.

### E. Infrastructure that makes any of the above DRC-exact

**21. Exact signed-distance clearance fields with the top-2 owners per cell.**
- Evaluate distances analytically from vector geometry (pads, tracks as capsules, vias as discs, board edge) at cell centres. Use a uniform bin grid so each cell tests about 5–20 primitives.
- Store (d₁, owner₁, d₂, owner₂) per cell. The clearance seen by net n is d₁ if owner₁ ≠ n, else d₂.
- Obstacles are therefore *not* quantised; only centrelines are.
- Per-class legality: cell legal for net n if clr_dist ≥ w_n/2 + clr(n, owner).
- Via legality: an AND over layers of clr_dist ≥ r_via + clr, stored as one extra 2-D "column" field.
- Rebuild each iteration from the current routes: about 1–2 ms at 12 M cells. Jump flooding is the alternative if the primitive count explodes.

**22. Exact geometry extraction with charged elastic bands.**
- Steps:
  1. Grid path.
  2. Run-length compress.
  3. String-pull (funnel or visibility), checking segments against the exact signed-distance field: a segment is legal if min over the segment of clr_dist ≥ w/2 + clr.
  4. Snap to 45° (a small LP per net, or greedy snapping).
  5. Optional relaxation: vertices feel tension (shortening) plus a short-range repulsion from other nets' signed-distance fields, with step size limited to half the local slack so topology cannot change.
- This relaxation is also a GPU-friendly push-and-shove for small DRC fixes.
- Finish with the CPU exact DRC and local re-route of the few failing spots with a finer grid.
- Rasterise with a conservative margin of about h/√2 on centrelines before string-pulling. At h = 0.125 mm that margin is 0.09 mm, a large share of a 0.2 mm clearance. **Use h ≤ 0.07 mm for the fine stage near dense pins,** or accept that 0.125 mm is a coarse stage only.

**23. The pour as background phase, with a resistor model: diffusion's proper home.**
- Pour region = free space eroded by the pour clearance, per layer, stitched by vias.
- **Connectivity:** GPU connected-component labelling per iteration counts islands and orphaned GND pads.
- **Quality:** solve Laplace on the pour region, with GND pads as current sources and a reference sink (or pairwise effective resistance). Current density |∇φ| marks bottlenecks.
- Add β·|∇φ|² to signal-net cell prices, so signals avoid cutting the plane's critical necks.
- Here the Laplace model *is* the physics, since return currents in a plane distribute this way. Effective resistance also serves as a continuous pour-connectivity score for ranking configurations (infinite means disconnected).
- Stitching vias go where islands are adjacent across layers.

**24. Population strategy for randomised configurations.**
- Randomise:
  - initial history prices (log-normal noise);
  - net and batch order;
  - start pin of each Steiner tree;
  - via cost (×[0.5, 2]);
  - per-layer direction preference;
  - territory bids.
- **Successive halving:** 256 configurations at coarse resolution for 10 iterations → keep 64 for 10 more → keep 16 for the fine stage → keep 2–4 for geometry and DRC.
- **Elite reuse (a cheap crossover):** initialise new configurations' history maps from a blend of elite congestion maps plus noise.

---

## 2. Where the analogy breaks (worth stating plainly)

1. **Dimensionality.** Fluids fill area and conserve mass. Traces are 1-D curves with prescribed width. Any volume-based model needs a projection or skeletonisation step, and that step is where DRC is won or lost.
2. **Energy.** Diffusion and Darcy minimise an L² energy, so flow spreads. Wires want L¹ (length). Only nonlinear dynamics (Physarum, p→∞) or a min-plus formulation (eikonal) give L¹.
3. **Surface tension favours blobs.** It removes exactly the thin structures you need.
4. **Inertia and momentum** have no counterpart in routing and cause overshoot.
5. **Interaction between nets is combinatorial packing.** No local PDE resolves who takes which channel. Dual prices (negotiation, auctions) do, and the continuum theory (Beckmann, Wardrop, Carlier–Jimenez–Santambrogio) justifies PathFinder rather than replacing it.
6. **Continuum optima are fractional** (split flows). Rounding or annealing (ε→0, β>1) is heuristic, which is why "many randomised runs, keep the best" is a genuinely good fit.

---

## 3. Top 5, the end-to-end pipeline, and a cost estimate

**Top 5:**
1. Min-plus-scan eikonal PathFinder with Mehlhorn Voronoi-Steiner (ideas 5, 9, 10).
2. Entropic congested-transport price updates (14), which also give smooth maps for the placer.
3. Shared-field colour territories with auction bids (6), as the global coordinator and diversity source.
4. Physarum / DMK at coarse resolution for multi-pin topology and locking (3, 4).
5. The pour as a resistor-plus-connected-components background phase (23).

Exact signed-distance fields (21) and elastic-band extraction (22) are required infrastructure, not options.

### Pipeline (wgpu/Vulkan)

**Example board:** 4 layers, 200 nets, 150×100 mm at h = 0.07 mm (about 2143 × 1428 ≈ 3.06 M cells per layer, 12.2 M nodes).

**Data layout.**
- One vec4 per (x,y) cell holds the 4 layers, so vertical relaxation stays inside a thread and reads are coalesced.
- Distances are u32 fixed-point (1/32 cell) with the parent direction in the low bits, which enables atomicMin.
- Raise `maxStorageBufferBindingSize` above wgpu's 128 MB default. Vulkan on a 1650 allows about 2 GB.

**Stage 0: static fields (once per placement).**
- Exact top-2 signed-distance field of pads, keepouts and board edge per layer (fp16 distance + u16 owner, ×2), plus the via column field: about 100 MB.
- Price maps (present and history, fp16): about 50 MB.
- Time: under 20 ms.

**Stage 1: coarse, batched configurations.**
- Grid: h_c = 0.5 mm → 300 × 200 × 4 = 240 k nodes. Edge capacities come from the fine signed-distance field (how many tracks of each class fit through each coarse edge).
- Per configuration, optionally run 30 rounds of territory auction (6) to set initial prices (about 1 ms per round at coarse size).
- Then run Jacobi PathFinder with all nets simultaneously:
  - Each net gets a Mehlhorn Voronoi-Steiner wave in its bbox (average ≈ 15 k nodes).
  - Only a random 30–50% of conflicting nets re-route per iteration (damping; undamped Jacobi negotiation oscillates).
  - Each net gets a small discount on its own previous route (stickiness).
  - Once a net is conflict-free for K = 3 iterations it is frozen (locked).
- Multi-pin nets with 4 or more pins optionally get 100 Physarum steps (β ≈ 1.3) to choose their tree topology.
- **Memory:** 200 × 15 k × 4 B = 12 MB per configuration, so about 64 configurations can run concurrently in under 1 GB.
- **Time per iteration** (all nets, one configuration): 3 M nodes × 24 directional passes (8 directions × about 3 rounds, warm-started) × about 12 B ≈ 0.9 GB of traffic ≈ **8–10 ms** at a realistic ~100 GB/s. Congestion scatter and back-trace (one thread per net) add about 1 ms.
- **Iterations:** about 40 to converge, so about 0.4 s per configuration.
- **With successive halving:** (256×10 + 64×10 + 16×20) × 10 ms ≈ **35 s**. Tiling in shared memory (32×32 blocks relaxed to convergence locally, as in block FIM) could cut this 2–5×.
- **Physarum option:** each step ≈ 200 × 15 k × (one multigrid V-cycle ≈ 8 passes × 30 B) ≈ 0.7 GB ≈ 6.5 ms. 300 steps ≈ 2 s per configuration, about 5× PathFinder. That is why it is restricted to multi-pin nets, top configurations only.

**Stage 2: fine corridors, top 16 configurations.**
- Corridor = coarse route dilated to about 1.5 mm, which is about 20 k nodes per layer-segment per net. Allowing for detours and layer changes, about 40 k nodes per net, so 200 nets ≈ 8 M nodes (32 MB of distance field).
- Per iteration:
  - rebuild the copper top-2 signed-distance field from the current routes (about 2 ms);
  - update present congestion with the per-class-pair clearance rule;
  - min-plus scans or active-list Bellman–Ford in the masked corridors (about 2× waste from masking): 16 M × 24 × 12 B ≈ 4.6 GB ≈ **40–45 ms**.
- Rip-up and re-route with the same damped Jacobi scheme, or coloured Gauss–Seidel: batches of nets with disjoint corridors are routed in parallel and batches run sequentially. That is GPU-parallel inside a batch and keeps PathFinder's convergence.
- About 15 iterations ≈ 0.7 s per configuration; 16 configurations ≈ **11 s**.
- Turn-penalised lifting (12) multiplies this by 8, so apply it only to the final 2 configurations or to congested nets: about +5 s.

**Stage 3: pour.**
- Per candidate: connected-component labelling of the pour region (about 5 ms), plus a multigrid Laplace solve for effective resistance and bottleneck density (about 20–50 ms at full resolution, or run it at 2h).
- Feed |∇φ|² back into the stage-2 prices of the top configurations for 2–3 more iterations.
- Place stitching vias.

**Stage 4: geometry (CPU/GPU, per net in parallel).**
- String-pull against the exact signed-distance field, snap to 45°, elastic-band relaxation, via snapping, then pour polygon generation. Tens of ms.
- CPU exact KiCad-rule DRC and local fix-ups (fine-grid re-route of failing windows): about 0.2–1 s.
- Score each configuration by, in order: unrouted nets, DRC count, pour islands and effective resistance, wirelength, vias.

**Total on a GTX 1650: about 50–60 s for a 256-configuration population, or about 3 s for a single configuration.**
- These are memory-bound estimates. The main uncertainties are dispatch overhead in wgpu (keep one indirect dispatch per pass covering all nets' tiles) and warp divergence along corridor edges.
- Fits in 4 GiB with a wide margin: static ~250 MB, coarse batch <1 GB, fine <0.5 GB.

---

## 4. Cheap experiment (NumPy/PyTorch, 3–4 days) to kill or confirm

**Day 0, about 1 hour. Diagnosis check.**
- Add a leak conductance to the existing Jacobi diffusion code and descend −ε·log u.
- Expect about 28 and about 21 cells on the wall test. That confirms the L²-vs-L¹ diagnosis.

**Day 1. Min-plus sweep engine in PyTorch.**
- On a row with prefix sums S of edge costs: d_new = S + cummin(d − S) along the axis. Use `torch.cummin`. Flip the array for reverse directions, skew it for diagonals, and take a min over layers with a via cost for the vertical step.
- Use float64 (or put a large finite cost on obstacles) to avoid inf − inf. The WGSL version uses the associative tropical-map scan, which has no subtraction.
- Iterate the 8 directions until nothing changes.
- **Validate:** exact equality with Dijkstra on 8-neighbour grids (29.3 → 21.0 on the wall case). Count rounds vs number of bends.

**Day 2. Parallel negotiation.**
- Rasterise 3–5 real KiCad boards (10–200 nets) at 0.1 mm, using per-class inflated obstacles (the exact signed-distance field can come later).
- Run three variants: (a) serial PathFinder using the same engine; (b) Jacobi, all nets per iteration; (c) damped Jacobi with 30% random re-route plus a self-discount; (d) coloured Gauss–Seidel batches with disjoint bboxes.
- Record completion rate, iterations, wirelength, vias, and oscillation (number of nets flipping per iteration).
- Then 64 random seeds per board: plot the best-of-k quality curve.

**Day 3. Field add-ons.**
- (i) Territory auction (6) as a price initialiser vs zero initialisation: iterations to converge and diversity (Jaccard distance between elite routings).
- (ii) Physarum with β ∈ {1, 1.3} on 3–6-pin nets vs the Mehlhorn Voronoi-Steiner tree: tree length and number of steps to converge.
- (iii) Resistor bottleneck prices for the pour: number of islands with and without them.

**Kill and confirm criteria:**
- **Confirm the core** if (c) or (d) reaches ≥ 95% of the serial completion within ≤ 2× the serial iteration count, and best-of-64 beats the serial router's single run.
- **If (c) and (d) both oscillate or stall,** keep the GPU only as a per-net wavefront engine (sequential nets, each GPU-parallel: about 0.3 s per iteration for 200 nets). That is still a large win over CPU A*.
- **Kill Physarum** if it needs more than about 300 Poisson solves to land within 5% of the Voronoi-Steiner length on multi-pin nets.
- **Kill territories** if auction initialisation does not reduce PathFinder iterations by at least 25% or does not increase elite diversity.

---

## Sources

Verified this session:
- [GAMER: GPU-Accelerated Maze Routing (Lin, Liu, Young, Wong; ICCAD 2021 / TCAD 2023)](https://ieeexplore.ieee.org/document/9643563/)
- [Tero, Kobayashi, Nakagaki 2007, J. Theor. Biol. 244:553](https://pubmed.ncbi.nlm.nih.gov/17069858/)
- [Tero et al. 2010, Science 327:439 (Tokyo rail)](https://www.science.org/doi/10.1126/science.1177894)
- [Bonifaci, Mehlhorn, Varma, "Physarum can compute shortest paths" (SODA 2012 / JTB)](https://arxiv.org/pdf/1106.0423)
- [Facca, Cardin, Putti, "Branching structures emerging from a continuous optimal transport model", JCP 447 (2021)](https://arxiv.org/abs/1811.12691)
- [Lonardi, Facca, Putti, De Bacco, Phys. Rev. Research 3, 043010 (2021)](https://link.aps.org/doi/10.1103/PhysRevResearch.3.043010)
- [Lonardi, Baptista, De Bacco, "Immiscible color flows in optimal transport networks", Front. Phys. 2023](https://www.frontiersin.org/journals/physics/articles/10.3389/fphy.2023.1089114/full)
- [Carlier, Jimenez, Santambrogio, SIAM J. Control Optim. 47 (2008)](https://arxiv.org/pdf/math/0612719)
- [Beckmann 1952, Econometrica 20(4)](https://www.econometricsociety.org/publications/econometrica/1952/10/01/continuous-model-transportation)
- [Xia 2003, "Optimal paths related to transport problems", Commun. Contemp. Math. 5(2)](https://www.math.ucdavis.edu/~qlxia/summary/transport.html)
- [Crane, Weischedel, Wardetzky 2013, "Geodesics in heat", ACM TOG 32(5)](https://dl.acm.org/doi/10.1145/2516971.2516977)
- [Zhao 2005, "A fast sweeping method for Eikonal equations", Math. Comp. 74](https://www.ams.org/journals/mcom/2005-74-250/S0025-5718-04-01678-3/viewer/)
- [Jeong, Whitaker 2008, fast iterative method, SIAM J. Sci. Comput. 30](https://www.researchgate.net/publication/220411914_A_Fast_Iterative_Method_for_Eikonal_Equations)
- [Davidson, Baxter, Garland, Owens 2014, Near-Far SSSP on GPU (IPDPS)](https://mgarland.org/papers/2014/sssp/)
- [Rong, Tan 2006, jump flooding (I3D)](https://www.comp.nus.edu.sg/~tants/jfa.html)
- [Mirebeau et al. 2023, curvature-penalised shortest paths on GPU, CCPE 35(2)](https://onlinelibrary.wiley.com/doi/10.1002/cpe.7472)
- [Esedoglu, Otto 2015, threshold dynamics for networks with arbitrary surface tensions, CPAM 68(5)](https://www.researchgate.net/publication/263583836_Threshold_Dynamics_for_Networks_with_Arbitrary_Surface_Tensions)
- [Jacobs, Merkurjev, Esedoglu 2018, auction dynamics, JCP 354](https://dblp.uni-trier.de/rec/journals/jcphy/JacobsME18.html)
- [Evans, Gangbo 1999, Memoirs AMS 137 (p→∞ and Monge–Kantorovich)](https://bookstore.ams.org/memo-137-653/)
- [Mehlhorn 1988, IPL 27:125 (Voronoi-based 2-approximation Steiner tree)](https://www.sciencedirect.com/science/article/abs/pii/002001908890066X)
- [Wang, Lazarov, Sigmund 2011, robust projection in topology optimisation, SMO 43](https://link.springer.com/article/10.1007/s00158-010-0602-y)
- [Niemeyer, Pietronero, Wiesmann 1984, dielectric breakdown model, PRL 52:1033](https://link.aps.org/doi/10.1103/PhysRevLett.52.1033)
- [Shan, Chen 1993, multi-component lattice Boltzmann, PRE 47:1815](https://www.researchgate.net/publication/13329506_Lattice_Boltzmann_Model_for_Simulating_Flows_with_Multiple_Phases_and_Components)
- [Cuturi 2013, Sinkhorn distances (NIPS)](https://papers.nips.cc/paper/4927-sinkhorn-distances-lightspeed-computation-of-optimal-transport)
- [DGR: Differentiable Global Router (DAC 2024)](https://dl.acm.org/doi/10.1145/3649329.3656530), GPU concurrent routing via relaxation; relevant to idea 14.

Not re-verified this session (classics or my recollection):
- Varadhan's formula.
- Akers' 2-bit Lee coding.
- The sparse top-K phase-field trick for grain growth.
- ePlace / DREAMPlace (electrostatic GPU placement).
- The oscillation of Jacobi-style parallel PathFinder, and the damping and colouring remedies. This is my engineering judgement, not a cited result.
- GPU connected-component labelling algorithms (e.g. Playne–Hawick).
