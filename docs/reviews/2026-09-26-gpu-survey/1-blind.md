# GPU-dominant PCB routing and placement: 21 approaches, a top 5, and one full pipeline

**How to read the references.** Links are to sources I found by web search in this session. A **†** means I cited it from memory and did not re-check it here. I believe these are correct, but check them before quoting. arXiv was blocked by the proxy, so arXiv items were confirmed through search listings and not full text.

**Ratings.** Each approach gets three numbers:
- **V** is value, from 1 to 5.
- **E** is effort, from 1 to 5, where higher means more work.
- **G%** is the share of total runtime spent on the GPU.

**Scale used for the numbers.** The worst case is a 150×100 mm board with 4 layers. At a fine cell size h = 0.05 mm that is 6 M cells per layer. At a coarse cell size H = 0.4 mm it is 94 k cells per layer. Bandwidth figures assume a GTX 1650 at 128 GB/s. One full sweep over a 2-layer field stored as f16 (half-precision float) reads and writes about 24 MB, which takes about 0.2 ms.

---

## 0. Shared substrate (the entries below refer to these as S1–S7)

**S1 – Two resolutions.**
- The coarse level (H ≈ 0.4–0.5 mm) is where you get quantity. At about 0.1–0.4 MB per layer per instance, hundreds of instances run as a batch.
- The fine level (h = 0.05 mm) is where you get quality. It is only ever computed inside windows or corridors, never over the whole board for every net.
- This is the single most important memory trick: run many instances at coarse resolution and only a few at fine resolution.

**S2 – Copper-owner distance field.** This one field makes clearance work for any mix of track widths and net classes.
- Per layer and per cell, store the nearest two *distinct* copper owners with their distances: `(owner1:u16, d1:f16, owner2:u16, d2:f16)`. That is 8 bytes per cell, so 6 M × 4 layers × 8 B = 192 MB at worst, or 96 MB for 2 layers.
- **Legality test for a track centerline of net n with width w at cell c:** take d = d1 if owner1 ≠ n, otherwise d2. The cell is legal if d ≥ w/2 + clr(n, owner) + δ, where δ = h/√2 is a guard band for rounding.
- One field serves every net class. The second owner is what stops a net's own copper from blocking it.
- **How to build it:** either compute exact distances to capsules and polygons after binning primitives into tiles (the way GPU vector renderers work), or use jump flooding ([Rong & Tan, I3D 2006](https://dl.acm.org/doi/10.1145/1111411.1111431)). Jump flooding is approximate; the exact tile-binned version is preferable for DRC safety.
- **Updates:** when copper changes, re-stamp only the affected tiles.

**S3 – Window atlas.** Each net gets a window: its bounding box plus a margin, or later a corridor. Windows are shelf-packed into one 2-D atlas in units of 32×32-cell tiles.
- One workgroup handles one tile. Tiles are small enough for shared memory: 64×64 cells in f16 is 8 KB, within WebGPU's default 16 KB per workgroup.
- Pour nets (GND, VCC) never get a full-board window at fine resolution.

**S4 – Vias.** The grid is 3-D, with the layer as the third axis.
- A per-class via-legal bitmap marks cells where min over layers of the distance ≥ r_via + clr, which covers through-hole vias.
- A layer change costs c_via.

**S5 – Pour engine.** The pour region for a net is the zone minus everything closer than clearance to foreign copper, i.e. `zone ∩ {d_foreign ≥ clr}`. Then:
- run GPU connected-component labelling ([Playne & Hawick, TPDS 2018](https://www.semanticscholar.org/paper/A-New-Algorithm-for-Parallel-Connected-Component-on-Playne-Hawick/83b18472014b67337ebabde71f6f14c28e220b7c));
- remove islands;
- check that every pour-net pad touches the main component;
- propose stitching vias where both layers' pours of the same net overlap on via-legal cells.
- **Scalar outputs:** the number of pour components and the pour-net pads left disconnected. This takes milliseconds per instance, so pour shredding becomes a term every candidate can be scored on.

**S6 – Vectorizer** (grid path to exact geometry).
1. Merge the grid chain into runs of 0°/45°/90° segments.
2. String-pull on the GPU: each vertex tries to jump to the furthest later vertex it can see. Visibility is checked by marching along the segment through the S2 field, stepping by the distance to the nearest obstacle (sphere tracing, as in ray-marching renderers), with clearance requirement w/2 + clr. All vertices run in parallel.
3. Snap to 45° angles and merge collinear segments.
4. On the CPU: build exact polygons (Clipper-style offsetting), run exact DRC, and apply local fixes.

**S7 – Batch dimension.** Every buffer has an instance index as its leading dimension, where an instance is one seed or one placement. Memory decides the batch size B: about 256 or more at coarse resolution, about 10–30 at fine resolution with windows, and about 4–10 at fine resolution over the full board.

**Kernel conventions throughout:**
- For shortest-path relaxation, pack `(dist:u16 quantized << 16) | parent_dir` into a single u32 and use `atomicMin`. WGSL only has atomics on 32-bit integers, and this gets both distance and parent in one atomic.
- Use shader-f16 for storage where the device supports it; check this on the 1650.
- Use subgroup operations for frontier compaction where wgpu exposes them; check availability.

---

## A. Negotiated congestion made concurrent

### 1. Batched ("Jacobi") PathFinder over a window atlas
**Origin.** PathFinder negotiated congestion ([McMurchie & Ebeling 1995]†). GPU routers that already do this include [Corolla (FPGA'17)](https://dl.acm.org/doi/10.1145/3020078.3021732), [GGR (ICCAD'22)](https://dl.acm.org/doi/10.1145/3508352.3549474), [FastGR (DATE'22 / TCAD)](https://www.cse.cuhk.edu.hk/~byu/papers/J86-TCAD2023-FastGR.pdf) and [InstantGR (ICCAD'24, open source)](https://github.com/cuhk-eda/InstantGR). The shortest-path primitive is [GAMER](https://dl.acm.org/doi/10.1109/ICCAD51958.2021.9643563): alternating horizontal and vertical sweeps, each a parallel scan taking O(log n) on an n×n grid.

**How it maps.** Each iteration:
1. Freeze the cost map: `present(overuse) × history + base`.
2. Pick a random fraction p of nets, say 10–30%. If you want no interference at all, pick a set of nets whose windows don't overlap (graph colouring), as InstantGR does.
3. Run shortest paths in each chosen net's window. A GAMER sweep costs one kernel per direction, and 4–12 alternating sweeps usually converge; add a few more for mazes.
4. Backtrace in parallel with one thread per net. Paths are 1–4 k cells.
5. Rip up the old paths and stamp the new ones into the occupancy map with atomics.
6. Update history costs.

Expect 30–100 iterations, each a handful of kernel launches per sweep.

- **(a) Multi-pin nets.** Grow the tree inside the kernel: after reaching a pin, reseed a multi-source wavefront from the whole tree. That is k−1 wavefronts per net. Entry 8 gives optimal Steiner trees for small k.
- **(b) Competition for space.** Occupancy counts demand. Clearance is enforced by stamping each path's footprint, dilated by w/2 + clr, into per-class conflict maps, or by checking against S2 against the current copper. The history term decides who yields at a bottleneck.
- **(c) Vias.** 3-D grid with via cost (S4).
- **(d) Pours.** The pour net is not routed as tracks. Add a "pour-criticality" cost: high where the pour region is thin, e.g. inverse of the distance-to-edge of the pour region, and on estimated cut vertices. Rescore with S5 every N iterations.
- **(e) Geometry.** S6.

**GPU share:** about 90–95%.

**Memory:**
- Per instance: the cost/occupancy map at about 4 B per cell, which is 48 MB for a 2-layer board at fine resolution.
- Per net: an f16 distance plus a u8 parent over its window. The sum of window areas is typically 3–10× the board area, so 50–150 MB.

**Quality.** Near serial PathFinder once converged, but it needs 1.5–3× more iterations. Updating all nets at once (Jacobi) oscillates; routing a random subset and damping fixes that.

**Many runs.** The seed controls the subset schedule and adds cost noise. Run B instances in a batch at coarse resolution.

**Risk.** Oscillation on dense 2-layer boards. Also, nets spanning the whole board have huge windows; route those at coarse resolution or treat them as pours.

**First experiment.** In NumPy/SciPy, on your existing test boards at 0.1 mm: route each net with `scipy.sparse.csgraph.dijkstra` against a frozen cost map. Compare p ∈ {1, 0.3, 0.1} against your sequential router on completion rate, via count and number of iterations.

**V5 E3 G90.**

### 2. Bit-parallel Lee wavefronts (Akers coding, bit-sliced across nets)
**Origin.** Lee 1961†. Akers 1967† showed backtrace needs only 2 bits per cell (the sequence 1,1,2,2). The bit packing borrows from chess bitboards and bit-parallel morphology. The Steiner heuristic comes from [Mehlhorn 1988]†: growing Voronoi regions from each pin and merging them gives a 2-approximate Steiner tree.

**How it maps.**
- Reachable sets are packed 32 cells per u32 along x. One step is `R' = (R | R≪1 | R≫1 | R_up | R_dn | (R_otherLayer & viaOK)) & free_class | own_copper`.
- Each workgroup keeps a 64×64 tile in shared memory and advances 16–32 steps per launch (temporal blocking), then exchanges halos.
- Alternatively, slice by nets: one u32 per cell holds 32 nets of the same class, so one instruction advances 32 wavefronts.
- To backtrace, save the reachable set every 32 steps and re-expand locally, or keep the 2-bit Akers phase.
- Cost can be approximated with gating: a cell of cost k admits the wave only when `step % k == 0`, or with probability 1/k. The probabilistic version gives a random tie-break for free.

- **(a) Multi-pin.** Flood from all pins at once, each labelled. When fronts from two components meet, merge them with union-find and backtrace the join (Mehlhorn).
- **(b) Competition.** Floods run simultaneously; overlaps are settled by entry 1 or by random priority.
- **(c) Vias.** One bit-plane per layer plus a via mask.
- **(d) Pours.** Pour connectivity analysis is the same flood kernel.
- **(e) Geometry.** Paths are 4- or 8-connected staircases, so S6 is mandatory.

**GPU share:** about 95%.

**Memory.** 1 bit per cell per net: 6 M cells × 2 layers × 400 nets / 8 = 600 MB for full-board fields for all nets, or 60–150 MB with windows. Akers coding doubles that.

**Quality.** The cheapest wavefront available. It ignores bend and via costs (gating approximates them) and the paths look ugly before S6. As a feasibility oracle it is excellent.

**Many runs.** Probabilistic gating gives hundreds of distinct route sets.

**Risk.** Unit cost is too crude to minimise vias.

**First experiment.** A NumPy uint64 bitboard flood on one board. Compare path lengths against Dijkstra and measure the number of steps.

**V4 E2 G95.**

### 3. Eikonal / geodesic fields with an octilinear (Finsler) metric
**Origin.** The fast iterative method ([Jeong & Whitaker, SISC 2008](https://www.researchgate.net/publication/220411914_A_Fast_Iterative_Method_for_Eikonal_Equations); the search result I found says 50–200× faster than serial fast marching) and fast sweeping (Zhao 2005†). The idea is borrowed from seismic travel-time computation and robot planning.

**How it maps.**
- Solve |∇T|_F = cost(x) in each net's window. Using an anisotropic norm whose unit ball is an octagon makes geodesics come out octilinear.
- An active-list kernel updates only cells in the narrow band, then extract the path by gradient descent on T.
- The result is continuous and any-angle to sub-cell accuracy. This is the most human-looking raw output and the easiest to vectorise.
- **(a)** Pins as sources, then entry 8. **(b)** Cost comes from S2 and occupancy; negotiation as in entry 1. **(c)** Layers are coupled at via cells with a jump cost. **(d)** As in entry 1. **(e)** Nearly direct: gradient-descent polylines go straight into S6.

**GPU share:** about 95%. **Memory:** one f16 field per net window.

**Quality.** Paths are shorter and smoother than 8-connected A*.

**Risk.** Gradient descent near obstacles when the corridor is only about one track wide. Fall back to discrete backtrace there.

**First experiment.** scikit-fmm† with a speed map taken from your obstacle raster. Compare against A* after vectorising both.

**V3 E2 G95.** This is really a better primitive to use inside entry 1 than a standalone router.

### 4. Coarse-to-fine: concurrent global routing, then detailed routing in corridors
**Origin.** The standard VLSI split between global routing (GR) and detailed routing (DR). [DGR (DAC'24)](https://dl.acm.org/doi/10.1145/3649329.3656530) makes GR fully concurrent and differentiable on the GPU: a DAG forest of Steiner and pattern candidates with soft selection. GGR and InstantGR do batched maze-plus-pattern routing on the GPU.

**How it maps.**
- **Coarse level.** Use 0.4–0.5 mm "GCells", 2 or 4 layers, with each GCell edge's capacity computed from S2. Capacity is the number of tracks of each class that fit through the gap between obstacles along that edge; the GPU computes it per edge.
- **Global routing.** Candidates are L, Z and 3-bend patterns plus a maze fallback. Solve either as a DGR-style softmax over candidates optimised with Adam on overflow + wirelength + vias, or with entry 5.
- **Detailed routing.** Each net's corridor is its GR path dilated by 1–2 GCells, and becomes its S3 window at h = 0.05 mm, where entry 1 runs.
- **(a)** RSMT candidates such as FLUTE† at coarse resolution. **(b)** GCell capacity overflow. **(c)** Via capacity per GCell. **(d)** Pour nets consume capacity only to keep a backbone: reserve a minimum-width channel so the pour stays connected. **(e)** S6.
- Pin access for fine-pitch parts is not modelled at the coarse level; entry 16 handles it.

**GPU share:** 85–95%.

**Memory.** Coarse: well under 5 MB per instance, so B ≥ 256. Fine: corridors are about 5% of the board, around 20 k cells per net.

**Quality.** Global decisions, i.e. which net yields where, are better than a sequential router's. The risk is that a corridor turns out infeasible at detailed level; widen it and re-negotiate.

**Many runs.** Run hundreds of global-routing seeds, send the best 10–20 to detailed routing.

**Risk.** Capacity estimates are poor next to 0.4 mm-pitch pads.

**First experiment.** Build the GCell capacity map from your raster and run a PyTorch softmax-over-patterns GR (a DGR-lite). Check correlation between coarse overflow and whether detailed routing succeeds.

**V5 E3 G90.**

---

## B. Convex relaxations and flows

### 5. Soft-min path marginals + multiplicative-weights prices + Gibbs sampling ← strongest "global yield" mechanism
**Origin.** Smoothed dynamic programming ([Mensch & Blondel, ICML'18](https://proceedings.mlr.press/v80/mensch18a/mensch18a.pdf)); Garg–Könemann multiplicative weights for fractional multicommodity flow†; randomized rounding (Raghavan & Thompson 1987)†.

**How it maps.** Each iteration:
1. For every net in its window, compute a forward soft distance F from the source pins and a backward soft distance Bk from the sink pins. These are Bellman-Ford passes with log-sum-exp in place of min, again as GAMER-style sweeps.
2. The net's occupancy marginal is μ_n(c) = exp(−(F + Bk − F(t)) / T).
3. Demand is D = Σ_n (μ_n ⊛ disk(w_n/2 + clr)), i.e. each marginal convolved with its width-plus-clearance footprint, which is separable or done by FFT.
4. Update prices: p ← p · exp(η · (D/cap − 1)₊).
5. Anneal T downward and repeat, typically 30–100 iterations.

Then draw R discrete routings with a stochastic backtrace: at each step choose the predecessor with probability ∝ exp(−Δ/T). Finally legalise with a few iterations of entry 1.

- **(a)** Multi-pin is the weak spot. Chain pins in the coarse MST order, or use a soft Dreyfus–Wagner for k ≤ 4 (entry 8).
- **(b)** Handled natively and globally: prices settle who yields at a bottleneck, and no routing order is involved.
- **(c)** 3-D grid. **(d)** Give each pour net a soft wide-trace backbone so its area demand is priced. **(e)** Sample, legalise, then S6.

**GPU share:** about 97%.

**Memory.** F and Bk as f16 per net window, doubling entry 1's per-net memory. Best done at coarse resolution, where it is only a few MB per instance.

**Quality.** The fractional optimum is a lower bound and a very good global guide. After rounding and legalisation it should match or beat order-dependent sequential routers on bottleneck boards.

**Many runs.** Built in: one converged field yields hundreds of samples, since sampling is cheap.

**Risks:**
- f16 underflow in log-sum-exp; subtract a per-tile maximum.
- Freezing as T → 0.

**First experiment.** In NumPy on the coarse grid of a hard 2-layer board: soft Bellman-Ford with `logsumexp` over stencils, prices updated by multiplicative weights. Plot overflow per iteration, and check how often sampled routings legalise within 5 PathFinder iterations.

**V5 E3 G97.**

### 6. Fractional multicommodity-flow LP solved by PDHG, then rounded
**Origin.** PDLP / [cuPDLP.jl](https://arxiv.org/abs/2311.12180) / cuPDLPx: restarted primal-dual hybrid gradient (PDHG), which is competitive with Gurobi on GPU for large LPs.

**How it maps.**
- Edge-flow variables x_{n,e} over each net's window graph, with flow conservation (the divergence operator is a grid stencil, so no sparse matrix is needed) and shared capacity constraints.
- PDHG then needs only stencil matrix-vector products, one kernel pair per iteration, run for thousands of iterations.
- Round by path decomposition plus randomized rounding.
- **(a)** A Steiner LP needs one commodity per sink with x_e ≥ f^k_e, which blows up in size; better to decompose into 2-pin connections.
- **(b)** Exact capacity constraints; widths are handled through capacity.
- **(c)(d)(e)** As in entry 5.

**GPU share:** about 99%.

**Memory.** At coarse resolution about 94 k × 4 edges × L × nets × (fraction of board in the window), roughly 10–50 M f32 values, which is 40–200 MB.

**Quality.** An exact relaxation bound, which is useful to diagnose whether a board is routable at all. It overlaps heavily with entry 5, which is simpler.

**Risk.** Slow convergence to high accuracy, and path decomposition is fiddly.

**First experiment.** cuPDLP or HiGHS† on the coarse LP of a single board. Compare its bound against what entry 5 achieves.

**V3 E4 G99.** Mostly worth it as a routability oracle for placement.

### 7. Max-sum belief propagation for Steiner-tree packing
**Origin.** [Bayati et al., PRL 2008](https://arxiv.org/pdf/0807.3373) (cavity method for Steiner trees) and [Braunstein & Muntoni 2017](https://arxiv.org/abs/1712.07041). The latter handles vertex-disjoint and edge-disjoint *packing* of several trees, and on VLSI benchmarks with known optima it came within 4% of optimum, reaching it in two cases.

**How it maps.** Each cell's state is (net ∈ candidates, parent direction, depth ≤ D). Messages pass along grid edges and are updated synchronously as stencils, with damping and reinforcement.
- Candidate sets must be restricted to at most 4 nets per cell using the coarse routing, otherwise state space explodes.
- **(a)** Native Steiner trees. **(b)** Native disjointness, but only on a uniform *track grid* whose pitch is at least width + clearance. Mixed widths on a fine grid need exclusion neighbourhoods and become hard. **(c)** Layer as a dimension. **(d)** Poor fit. **(e)** Track-grid paths go to S6.

**GPU share:** about 99%. **Memory:** states × directions × cells, only feasible on a coarse or track grid.

**Quality.** Potentially excellent on uniform-pitch areas.

**Risk.** Research-grade; convergence with many nets is unclear.

**First experiment.** Run Braunstein's msgsteiner code† on a coarse track-grid version of one board.

**V3 E5 G99.**

### 8. Dreyfus–Wagner Steiner trees as image algebra
**Origin.** Dreyfus–Wagner 1971†, with the Erickson–Monma–Veinott form†.

**How it maps.**
- Keep one field S[X] for each subset X of terminals, where S[X](c) is the cost of the cheapest tree connecting X to cell c.
- Recurrence: S[X] = SSSP-relax( min over splits Y⊂X of (S[Y] + S[X∖Y]) ).
- The min over splits is element-wise, so perfectly parallel. The relaxation is the same kernel as in entries 1 and 3.
- Cost: 3^k element-wise passes and 2^k shortest-path relaxations. For k ≤ 6 that is 729 and 64.
- For large k: nets with k > 6 go to a pour, or to Mehlhorn's Voronoi heuristic (entry 2), or get clustered with DW run over clusters.
- **(a)** Solved optimally for the given cost map, which is better than growing the tree pin by pin (Prim–Dijkstra style). **(b)–(e)** Inherited from the host router.

**GPU share:** about 99%. **Memory:** 2^k fields per net window, so process nets in slices.

**First experiment.** NumPy DW on the 3–5-pin nets. Measure wirelength and via savings against sequential tree growth.

**V4 E2 G99.** A cheap, strict improvement for multi-pin nets.

### 9. Escape routing and bottleneck allocation as min-cost flow (GPU auction or push-relabel)
**Origin.** Network-flow escape routing for BGAs and fine-pitch parts is well established in the literature, but I did not check a specific paper. Solvers: Bertsekas' auction algorithm†; GPU push-relabel from GPU graph cuts (Vineet & Narayanan 2008†).

**How it maps.**
- **Escapes.** For each fine-pitch part, build a small graph: pads → gaps between pads, with capacity floor((gap − clr) / (w + clr)) → optional via or dog-bone nodes → a ring of escape points. One workgroup solves one part × one instance.
- **Output:** each pad's escape direction and layer, plus a dog-bone if needed. These become fixed terminals for global routing. This decides fine-pitch escape instead of hoping the maze router finds it.
- **Bottlenecks.** Compute channel capacities between obstacle pairs, e.g. on a Delaunay triangulation of obstacles as in topological routers. They feed entry 4's capacities.
- **(b)** Capacities are computed per width class. **(c)** Explicit via nodes. **(d)(e)** Not applicable here.

**GPU share:** 70–90%. The graphs are tiny, so doing this on the CPU is also fine.

**Quality.** Fixes a named hard part cheaply.

**Risk.** Escape choices that look fine locally can be bad globally; mitigate by sampling several escape plans per instance.

**First experiment.** networkx min-cost flow on your 0.4–0.5 mm QFN/QFP footprints.

**V4 E3 G80.**

---

## C. Fields, physics and region growing

### 10. Physarum Kirchhoff-flow networks (harmonic potentials as a special case)
**Origin.** [Tero, Kobayashi & Nakagaki, J. Theor. Biol. 2007](https://pubmed.ncbi.nlm.nih.gov/22732274/) (the model converges to the shortest path); a [Physarum approach to Steiner trees](https://arxiv.org/pdf/1903.08926); Tero et al., Science 2010, the Tokyo rail network†; harmonic-function path planning, which has no local minima (Connolly & Grupen 1993)†.

**How it maps.**
- Each net n has a conductance field σ_n. Solve ∇·(σ_n ∇φ_n) = s_n with multigrid or Jacobi-preconditioned CG in the window. The source is one pin and the other pins are equal sinks; rotate which pin is the source every iteration (Tero's Steiner variant).
- Flux q = σ|∇φ|. Update dσ_n/dt = f(|q_n|) − σ_n − λ·Σ_{m≠n} (σ_m ⊛ disk), the last term being the competition between nets.
- Run 100–500 outer iterations, each needing about 10 V-cycles. Threshold the tubes, skeletonise, vectorise.
- **(a)** Native Steiner-like trees. **(b)** Through the competition term; widths via the disk kernel. **(c)** Vertical conductance at via-legal cells, lower than in-plane. **(d)** Pours are just high-conductance regions whose connectivity is checked. **(e)** Skeleton, then S6.

**GPU share:** about 98%.

**Memory.** σ and φ in f16 per net window, plus a multigrid hierarchy (+33%).

**Quality.** Organic, loopy, and slow to converge; worse than entry 1 on dense boards.

**Many runs.** Random initial σ.

**Risk.** Convergence time; turning tubes into exact tracks.

**First experiment.** SciPy sparse solves on a coarse 2-layer grid with 10 nets.

**V3 E3 G98.** The Poisson solver could be reused as a routability or congestion predictor.

### 11. Continuous multi-label Potts: allocate regions to nets first, then route inside them
**Origin.** Convex relaxation of minimal partitions (Pock, Cremers, Bischof & Chambolle 2009†), the Chambolle–Pock PDHG solver†, connectivity priors (Nowozin & Lampert 2009†). The routing analogue is topological routing: sketch who goes where before fixing geometry.

**How it maps.**
- Labels per cell are the nets plus "free". The data term is the geodesic distance to that net's pins (from entry 3). Total variation encourages compact, human-like territories.
- Restrict to at most 8 candidate labels per cell, taken from coarse routing.
- Enforce connectivity afterwards: keep the component containing the pins, then repair.
- Route each net inside its own territory with no interference, which is embarrassingly parallel.
- Pour nets automatically receive the leftover territories, so pours are first-class.
- **(b)** Test that each territory can hold a track by eroding it by w/2 + clr. **(c)** Labels per layer, with a via term. **(e)** Route inside territory, then S6.

**GPU share:** about 95%. **Memory:** 8 labels × f16 × cells, about 100 MB at fine resolution on 2 layers.

**Quality.** Unknown. Pours come out nicely; signal routes can be wasteful.

**Risk.** Connectivity is not convex.

**First experiment.** PyTorch PDHG with 5 labels on a crop.

**V3 E4 G95.**

### 12. Neural cellular automaton router
**Origin.** "Growing Neural Cellular Automata", Distill 2020 (Mordvintsev et al.)†.

**How it maps.** Each cell holds about 16 state channels: per-net logits for a few candidate nets, a direction, and wave signals. A 3×3 convolution plus an MLP updates each cell, iterated for T steps. That is one WGSL kernel per step, the ideal GPU workload.
- Train with backpropagation through time or evolution strategies on synthetic boards, with losses for connectivity, overlap and length. Distil from the best of N solutions produced by other methods.
- **(a)(b)** Learned. **(c)** Layer channels. **(d)** A pour channel. **(e)** Threshold, then S6.

**GPU share:** about 99%. **Memory:** 16 channels × f16 × cells, about 200 MB at fine resolution on 2 layers; realistically run at coarse resolution.

**Quality.** Probably below entry 1, and hard constraints still need legalising afterwards.

**Risk.** Generalisation and training infrastructure.

**First experiment.** A small NCA that connects 2–3 pin pairs on 64×64 grids.

**V2 E4 G99.**

---

## D. Populations, sampling, stochastic search

### 13. Population of random priority orders, advanced in lock-step, with cross-entropy learning of orders ← most literal "quantity beats quality"
**Origin.** Prioritized planning from multi-agent path finding (MAPF) (Erdmann & Lozano-Pérez 1987†; CBS, Sharon et al. 2015†); the cross-entropy method with a Plackett–Luce distribution over permutations†.

**How it maps.**
- M instances each hold their own random net order and their own cost map.
- In wave i, every instance routes its i-th net. One kernel handles M windows at once with identical control flow: the same wavefront kernel, different data.
- Each instance is serial, but the GPU is full because there are M of them. The serial router's logic survives almost unchanged.
- After a generation, refit the order distribution to the top-k instances (cross-entropy method). Optionally allow a bounded amount of per-instance rip-up (entry 1 inside an instance).
- **(a)–(e)** Same as the serial router.

**GPU share:** about 90%.

**Memory.** Per instance: 0.3 MB per layer at coarse resolution, so M = 1024 is easy. At fine resolution, 24 MB per layer for the full board, so M ≈ 40–80; with windowed cost maps, more.

**Quality.** The best-of-M serial result, which beats a single serial run. The cross-entropy step targets exactly the "who yields" failure mode.

**Risk.** Late waves route long nets with little parallelism inside each instance. Sort nets so similar ones fall in the same wave.

**First experiment.** Run your current serial router with 200 random orders on CPU (as a proxy) and plot the distribution of results. If best-of-200 is much better than the median, this approach is justified.

**V4 E2 G90.**

### 14. Parallel tempering with checkerboard-parallel local rip-up moves
**Origin.** Metropolis updates on Ising/Potts models using a checkerboard; replica exchange (Swendsen & Wang 1986†, Geyer 1991†).

**How it maps.**
- The state is a complete, possibly illegal, routing.
- A move rips up the parts of nets that cross one 64×64 tile and reroutes them inside the tile with a wavefront in shared memory, pinned at the boundary crossings. Some moves toggle a via or swap which layer a segment uses.
- The energy is length + vias + overlap penalty + pour penalty from S5.
- Tiles of the same checkerboard colour are independent, so thousands of moves run concurrently. Replicas at different temperatures form the batch dimension, with periodic swaps.
- **(a)–(c)** Inside the tile. **(d)** Local pour-connectivity change via tile-level CCL. **(e)** S6.

**GPU share:** about 90%. **Memory:** B replicas × the fine per-instance map, about 50 MB each for 2 layers, so B ≈ 20–40.

**Quality.** A strong polisher (vias, length, pour repair). It cannot make fundamentally non-local changes on its own.

**First experiment.** A CPU prototype with tile moves on a converged routing from entry 1, measuring via reduction.

**V4 E3 G90.**

### 15. Wave Function Collapse / constraint propagation over a vocabulary of track tiles
**Origin.** Gumin's WFC (2016)† and Merrell's model synthesis (2007)†.

**How it maps.**
- Tiles on a routing pitch carry edge labels (net, layer). Arc-consistency elimination runs in parallel (Jacobi) per cell, and many distant cells can be collapsed at once.
- On contradiction, restart with a new seed; that is the "quantity" lever.
- Global connectivity is not a local constraint, so a guidance field from entry 5 is needed.
- The best use is escape-pattern synthesis for fine-pitch footprints: a pre-generated pattern library per footprint and rule set.

**GPU share:** about 90%.

**Quality.** Poor as a general router.

**First experiment.** A NumPy WFC for escapes of one 0.5 mm QFN.

**V2 E3 G90.**

---

## E. Learned components

### 16. A learned corridor/congestion prior that biases the costs, trained on your own best-of-N runs
**Origin.** [Neural A* (ICML'21)](https://github.com/omron-sinicx/neural-astar) and [TransPath (AAAI'23)](https://dl.acm.org/doi/10.1609/aaai.v37i10.26465) learn guidance maps for search. [PRNet / HubRouter (NeurIPS'23)](https://papers.neurips.cc/paper_files/paper/2023/file/f7f98663c516fceb582354ee2d9d274d-Paper-Conference.pdf) generate global routes; HubRouter's summary notes PRNet's routes can come out disconnected, which HubRouter's hub-plus-connection scheme avoids. On the PCB side: [MCTS + DNN circuit routing (He & Bao 2020)](https://arxiv.org/abs/2006.13607), [DreamerV3 + FreeRouting (2026)](https://www.sciencedirect.com/science/article/abs/pii/S0957417426003374), and [DeepPCB](https://deeppcb.ai/) (commercial, reinforcement-learning based).

**How it maps.**
- A U-Net or small diffusion model takes the rasterised board (pads, nets, obstacles) and predicts per-net corridor heatmaps and a congestion map.
- Its output is a cost bias fed into entries 1, 4 and 5; it never produces geometry directly, so correctness never depends on it.
- The GPU pipeline produces training data as a by-product (self-distillation or expert iteration).
- A diffusion model's samples give diversity, i.e. many runs.

**GPU share:** about 99% for inference. **Memory:** a model of about 10–50 M parameters, well under 200 MB.

**Risks.** Little data diversity at first, and running inference under wgpu; ONNX-to-wgpu runtimes exist but are immature.

**First experiment.** After the other entries are running, log (board → final routes) pairs and train a U-Net corridor predictor in PyTorch. Measure how many negotiation iterations it saves.

**V3 E4 G99.** Value rises to about 4 over time.

---

## F. Getting from routes to exact geometry

### 17. Topological / rubber-band back-end
**Origin.** SURF (Dai, Dayan & Staepelaere, DAC 1991)†, Dayan's thesis 1997†, the commercial router TopoR; shortest homotopic paths (Hershberger & Snoeyink 1994)†.

**How it maps.**
- From any raster route, extract its homotopy class: the sequence of obstacles it passes on the left or right.
- Compute clearance-aware shortest paths in that class (rubber bands), then spread the wires apart.
- A GPU variant treats polyline vertices as particles under XPBD constraints (Macklin et al. 2016†): pull tight, stay out of the S2 field, keep segments octilinear. Hundreds of polylines relax in parallel.
- The result is exact geometry that uses clearance optimally, recovering the slack a raster wastes. It is the most human-looking output and can re-open space for failed nets.

**GPU share:** 50–80%.

**Quality.** It raises the output quality of every other entry.

**Risk.** Correct homotopy bookkeeping across vias.

**First experiment.** S6 string-pulling plus a Shapely/Clipper exact-DRC pass over routes from entry 1. Measure the length reduction and freed area.

**V4 E3–4 G60.**

### 18. Raster DRC / pour / SDF engine (S2 + S5 + S6) as a first-class component
This is the substrate section above treated as its own item. Everything relies on it, and it is what makes scoring hundreds of candidates, including pour connectivity, cost milliseconds each.

**V5 E2 G≈100** (except the final exact CPU DRC).

---

## G. Placement

### 19. Batched analytical placement with routability from the router's own fields
**Origin.** ePlace (electrostatic density)†, DREAMPlace (DAC'19)†, and [Cypress (ISPD'25 best paper)](https://github.com/NVlabs/Cypress): GPU placement specifically for PCBs, with a log-sum-exp wirelength that accounts for orientation.

**How it maps.**
- Batch B placements. Density is computed with FFT electrostatics on a per-side grid (top and bottom).
- Wirelength uses LSE or weighted-average smoothing.
- The routability term is either RUDY (Spindler & Johannes 2007)† or entry 5's coarse soft demand map, whose price gradient with respect to pad positions follows from the envelope theorem: move pads down the price field.
- Orientation is chosen as in Cypress.
- Legalisation uses entry 20 or a CPU Tetris/Abacus pass.

**GPU share:** about 90%. **Memory:** trivial, around 10 MB per instance.

**Quality.** Cypress reports large speedups (the search result says up to 492×) along with quality claims.

**First experiment.** Run Cypress's PyTorch code on your netlists. Compare the best of 32 seeds against your current placement by the router's completion rate.

**V5 E3 G90.**

### 20. Population placement search with routing in the loop
**Origin.** Simulated annealing / parallel tempering, CMA-ES†, MAP-Elites†.

**How it maps.**
- B = 128–512 individuals. Moves are shift, swap, rotate and flip-side.
- Overlap is evaluated by rasterising courtyards with atomics.
- **Fitness is an actual coarse GPU routing** (entry 4 or 5 at 0.5 mm, 10–20 iterations) plus S5 pour connectivity.
- Fixed components stay masked.
- XPBD rigid bodies (springs for nets, SDF collisions with courtyards and the board edge, Langevin noise) do legalisation and "compaction" in parallel across instances.

**GPU share:** about 90%. **Memory:** coarse routing at about 1–5 MB per individual.

**Quality.** It optimises what you actually care about, routability, rather than a proxy.

**Risk.** Coarse fitness is noisy; use a multi-fidelity schedule.

**First experiment.** Correlate coarse routing overflow with final router success over 50 random placements.

**V4 E3 G90.**

### 21. XPBD / rigid-body physics placer
Components are rigid bodies with courtyard SDF collisions, board-outline and cutout constraints, and star springs per net. Thousands of seeds run in parallel, with simulated-annealing temperature as noise. It can be the initialiser and legaliser for entries 19 and 20, and gives compact, human-like clusters.

**V3 E2 G95.**

---

## Top 5

1. **Batched Jacobi PathFinder in a window atlas, with GPU sweep-based shortest paths (entries 1 and 3).** The workhorse. It is proven in VLSI (GGR, InstantGR, Corolla) and keeps negotiated-congestion quality.
2. **Soft-min marginals + multiplicative-weights prices + Gibbs sampling at coarse resolution (entry 5).** This is how you decide globally which net yields at a bottleneck, and it produces hundreds of diverse candidates almost for free.
3. **The raster DRC/pour engine (entry 18: S2 two-owner distance field, S5 pour CCL, S6 sphere-traced vectoriser).** It enables everything else. Clearance handles mixed widths and classes with one field, pour shredding becomes a scalar score per candidate, and the output is near-DRC-clean before any CPU work.
4. **Coarse-to-fine corridors (entry 4) with flow-based escape planning (entry 9) and Dreyfus–Wagner multi-pin nets (entry 8).** This is what makes 0.05 mm resolution fit in 4 GB and addresses fine-pitch escape directly.
5. **Batched analytical placement (Cypress-style) plus a population with routing in the loop (entries 19 and 20).** Placement quality dominates routability, and batching placements is trivially GPU work.

Honourable mentions: entry 13 (random orders plus cross-entropy) is the cheapest way to get "quantity beats quality" on the existing serial logic. Entry 17 (rubber-band back-end) and entry 14 (parallel tempering) are the polishers.

---

## One end-to-end pipeline I'd bet on

The numbers assume a GTX 1650, a 100×80 mm 2-layer board, 150 nets and about 1500 pads. They are order-of-magnitude estimates, not measurements.

**P0 – Preprocessing (CPU, under 100 ms).**
- Parse the board, compute pad access points, and build per-class rule tables.
- Build escape flow graphs for each fine-pitch part.

**P1 – Placement (GPU, B = 64–256).**
1. Initialise with XPBD (entry 21).
2. Run analytical placement: FFT density, LSE wirelength, orientations, and entry 5's coarse demand map from the previous round as the routability term.
3. Legalise in parallel, then do a final exact overlap check on the CPU.
4. Keep the top 8–16 placements by coarse routing fitness (entry 20).

**P2 – Escapes (GPU or CPU, milliseconds).**
- Solve per-part min-cost flows (entry 9) and sample 2–4 escape plans per placement. Escape points become terminals.
- Pour nets become zones plus a reserved backbone. Nets with more than 6 pins that are not pours are clustered.

**P3 – Coarse global routing (GPU, 0.4 mm grid, 250×200×2 cells).**
- Run entry 5 for all nets: 50 multiplicative-weights iterations with soft Bellman-Ford sweeps in windows, width-aware demand by convolution, and gradual lowering of T.
- Draw R = 32 samples per (placement, escape plan) and legalise each with 5–10 coarse PathFinder iterations.
- Score each by overflow, vias, length and a coarse S5 pour score. Keep the top 10–20.
- Memory: under 10 MB per instance, so all instances fit at once.
- Time: roughly 10–50 ms per multiplicative-weights iteration across the batch, so a few seconds in total.

**P4 – Detailed routing (GPU, 0.05 mm, corridors).**
- Corridor = global route dilated by 1–2 GCells, then atlas-packed (S3). That is roughly 20–50 k cells per net and 3–8 M cells per instance.
- The S2 owner-distance field is maintained incrementally.
- Entry 1 with GAMER sweeps in shared-memory tiles, 64×64 with 8–16 steps fused per launch. Multi-pin nets with k ≤ 6 use entry 8. Rip-up iterations are 10–40.
- If a corridor is infeasible, widen it, or feed its cost back to P3 for that net and re-sample.
- Batch of 10–20 instances. Budget about 1 GB for fields and atlas plus about 100 MB of S2 per instance.
- Time: roughly 0.3–2 s per instance; the batch runs concurrently.

**P5 – Pours and polishing (GPU).**
- S5: pour fill, CCL, island removal, stitching vias. If a pour is shredded, add a penalty and run entry 14's parallel-tempering tile moves (vias, layer swaps, local reroutes), with pour connectivity in the energy function.
- Run a few hundred milliseconds per instance and pick the best 2–3.

**P6 – Geometry and verification (GPU then CPU).**
- GPU: S6 string-pulling, optionally XPBD rubber-band relaxation (entry 17).
- CPU: exact polygons with Clipper-style offsetting, a KiCad-equivalent DRC, and small fixes (nudge a segment, move a via, a one-net local reroute on CPU).
- About 0.5–2 s. This is well under 10% of wall time.

**Overall.** Expect 85–95% of runtime on the GPU. Peak memory is about 2–3 GB on a 4 GB card. The design is deliberately quantity-first at P1–P3, where instances are cheap, and quality-first at P4–P6, where memory limits the batch.

**Build order and go/no-go tests.** Each step's experiment decides whether to continue.
1. **S2/S5/S6 engine**, checked against KiCad DRC on existing routed boards.
2. **Entry 1 on the GPU at 0.1 mm**, compared against the serial router.
3. **Entry 5 at coarse resolution**: does it beat order-dependent routing on the dense 2-layer boards?
4. **Corridors plus 0.05 mm routing.**
5. **Placement batching.**

The cheapest early go/no-go is the entry 13 experiment: 200 random orders of the existing router on CPU. It tells you whether quantity really buys quality on your boards before any GPU code is written.

**Language and API.** Stay on wgpu/WGSL. The only strong reasons for CUDA would be using cuPDLP (entry 6) or learned models (entries 12 and 16) as-is.
- WGSL constraints: atomics only on u32/i32, so use packed u32 `atomicMin` for distance plus parent.
- WebGPU's default is 16 KB of shared memory per workgroup; native Vulkan can raise the limit.
- f16 storage and subgroup operations need the corresponding wgpu features; check both on Turing (the 1650).
