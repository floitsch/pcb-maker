# Differentiable and relaxation-based routing (and joint place+route) on a GTX 1650-class GPU

## 0. The main points up front

1. **Most of what "differentiable routing" means in practice is two things: a parallel shortest-path oracle and a smooth outer loop.** DGR, Garg–Könemann, Frank–Wolfe/traffic assignment, ADMM, Lagrangian ascent and perturbed optimizers all reduce to two steps. First, compute a distance field for every net under a shared congestion-dependent cost. Second, update the prices or probabilities with a cheap elementwise or convolution step. If you make the first step fast and batched, every method on the list becomes a small change to the second step.
2. **On a raster grid, the shortest-path oracle is a scan.** Along a row with per-cell cost c and prefix sum S, the one-directional relaxation is exact: `d_new = S + cummin(d_old − S)`. The soft version is `d_new = S − τ·logcumsumexp(−(d_old − S)/τ)`. Alternating ±x, ±y, (±diagonals) and via passes gives a GAMER-style router (Lin, Liu & Wong, ICCAD'21) that converges in about (number of bends + 1) rounds. Every net, window and configuration is independent. It is two parallel scans per line: bandwidth-bound and well suited to WGSL.
3. **Clearance-aware congestion is linear, so it is cheap.** "Usage seen by class k" = Σ_j conv(occupancy of class j, disc of radius w_k/2 + c_kj + w_j/2). With K ≤ 4 classes that is K² convolutions per iteration for the whole board. Each net subtracts its own contribution inside its window ("total minus self"). This is what makes all-nets-parallel negotiation with width and clearance practical.
4. **Keep the fine raster coarse (0.1–0.125 mm) and use a continuous stage for accuracy.** A continuous post-route legalization step (elastic polylines with signed-distance clearance terms, on the GPU) recovers what the raster loses. Going from 0.05 mm to 0.1 mm is 4× fewer cells, and that factor applies to everything below.
5. **The hundreds-of-runs goal should be a funnel, not hundreds of full runs.** Screen many placements with a cheap tile-level relaxation, which also gives a lower bound on overflow. Then generate several rounded corridor samples for each survivor. Run full fine negotiation only on the top ~16.

---

## 1. Ideas

Notation. N = number of nets (200), L = layers (4), fine grid G_f ≈ 1500×1000×4 = 6 M cells at 0.1 mm, tile grid G_t ≈ 150×100×4 = 60 k nodes at 1 mm, K = number of net classes, B = batch of configurations.

### I1. Batched scan-based wavefront oracle, hard and soft (the foundation)
- **Refs:** GAMER (Lin, Liu & Wong, ICCAD 2021 / TCAD 2022; sweeps reduced from O(n²) to O(log² n); verified). Fast sweeping (Zhao, Math. Comp. 2005; not re-verified). InstantGR (Lin & Wong, ICCAD 2024, GPU global routing; verified that it exists, internals not verified).
- **Formulation:** for each net n, a cost field c_n(x) = base + history h(x) + present-congestion penalty p(u_{−n}(x)) + stickiness (see I2). Directional passes use the cumsum/cummin identity above. Vias are a per-cell min across z. 45° moves are diagonal scans with c·√2. Multi-pin nets are handled by Prim-style tree growth: seed the distance field with the current tree, run to the nearest unreached pin, back-trace, repeat. Nets are batched, so the number of steps is the maximum pin count, not the sum.
- **Kernel shape:** one workgroup per (net window, line), with a subgroup scan per line. Windows are the net's bounding box plus a margin, or a corridor from the tile stage. Memory: dist fp32 + parent u8 per window cell. The cost field is shared, fp16, read-only.
- **Scale:** corridor windows ≈ 100 k cells per net × 200 ≈ 20 M cells. About 8 rounds × 5 passes × 12 B/cell ≈ 10 GB of traffic, which is ~0.1–0.2 s for a full reroute of all nets on a 1650.
- **GPU fraction:** ~100% of the search.
- **Quality:** each individual path is exactly optimal under its cost field, the same as A*. There are no heuristic errors.
- **Risk:** many-bend paths in mazes need many rounds. Mitigate with a coarse-level potential as a warm start and with "delta" early termination per window.
- **First experiment:** 30 lines of PyTorch with `torch.cummin` and `logcumsumexp` on an exported board raster, checked against the existing A* costs.
- **Ratings:** value 5, effort 2, GPU 5.

### I2. Jacobi/ADMM PathFinder: all nets in parallel, with a proximal term and randomized partial rerouting (**top**)
- **Refs:** PathFinder (McMurchie & Ebeling 1995; not re-verified). ADMM (Boyd et al. 2011). Proximal Jacobian multi-block ADMM (Deng, Lai, Peng & Yin, J. Sci. Comput. 2017; from memory, unverified). Corolla, GPU FPGA routing (verified that it exists).
- **Formulation:** minimize Σ_n c·x_n subject to Σ_n A_k x_n ≤ cap, where x_n are the path cells and A_k is the clearance-dilated footprint. Augmented Lagrangian: history h is the dual variable, updated by h += α·max(0, u − cap). The present penalty is the ρ-term. Each iteration, every net solves the shortest-path subproblem with cost c + h + ρ·(u_{−n} − cap)_+ + (μ/2)·[x ∉ previous path of n].
- **The part that matters:** naive Jacobi PathFinder (all nets rerouting simultaneously) oscillates, because all nets flee the same hot spot. Two proven fixes make it converge like Gauss–Seidel. First, the proximal "stickiness" term μ. Second, reroute only a random subset each iteration, for example 20–40% of the nets that currently have conflicts. Choosing the subset with probability ∝ that net's overflow works well. The random subset choice is also a natural source of diversity between runs.
- **GPU fraction:** ~95%. The CPU only does bookkeeping.
- **Quality:** expect roughly the same completion rate as serial PathFinder after 1.5–2× more iterations, with wirelength within ±3%. Iterations cost ~100× less.
- **Risk:** convergence on the last few hard conflicts. Fall back to serial A* on the CPU for the final <1% of nets inside small windows.
- **First experiment:** PyTorch with I1 + I11 on 3 boards. Sweep the reroute fraction {0.1, 0.3, 1.0} × μ {0, 1, 3} and measure iterations to zero overflow.
- **Ratings:** value 5, effort 3, GPU 5.

### I3. Tile-level multicommodity flow via Frank–Wolfe/MSA or Garg–Könemann with batched shortest-path oracles (**top**)
- **Refs:** Garg & Könemann (SIAM J. Comput. 2007; unverified here, standard). Frank–Wolfe for traffic assignment (LeBlanc et al. 1975; unverified). Wardrop/Beckmann equilibrium.
- **Formulation:** variables f_n ∈ conv(Steiner/path polytope of n) on the tile graph. Objective: Σ_e w_e·φ(u_e/cap_e) + λ·WL, where φ is softplus or exponential overflow. Each FW step: gradient = edge prices; linear-minimization oracle = all nets' shortest trees under those prices (I1 at tile resolution); step f ← (1−γ)f + γ·f_LMO. Garg–Könemann is the same loop with multiplicative prices exp(ε·u/cap), and it gives a (1+ε) max-congestion approximation.
- **Two by-products:** (a) every LMO path is stored in a per-net **column pool**, which feeds I4. (b) The duals are prices, which feed placement (I9).
- **Memory:** 60 k nodes × 200 nets × fp16 ≈ 24 MB per configuration. B = 64 configurations fit easily. About 30–50 FW iterations, ~5–20 ms each per configuration.
- **GPU fraction:** ~100%.
- **Quality:** a good globally balanced corridor assignment. It is not a detailed route by itself.
- **Risk:** Steiner-tree nets make the oracle a heuristic, so this is no longer exact FW. In practice that is fine.
- **Ratings:** value 4, effort 2, GPU 5.

### I4. DGR-style Gumbel-softmax selection over the generated column pool (**top**)
- **Refs:** DGR (Du et al., DAC 2024; routing DAG forest, Gumbel-softmax with temperature annealing and top-p, GPU; verified). Gumbel-softmax (Jang et al., Maddison et al. 2017).
- **Adaptation:** DGR's candidates are 2-D L/Z-pattern routes. Those are poor on PCBs with obstacles and keepouts. Use the I3 column pool instead, 4–16 obstacle-aware corridors per 2-pin segment, plus 2–4 Steiner topologies per multi-pin net (I16).
- **Variables:** logits θ_{n,k}. Expected usage U = Σ p_{n,k}·A_{n,k}, applied as sparse scatter-add. The loss is overflow(U) + WL + vias. Optimize with Adam over ~200 steps.
- **Diversity:** after annealing, draw S Gumbel samples per configuration. That gives S discrete corridor assignments in one pass, each seeding I2.
- **Memory:** pool of 200 × 16 × ~300 tile-edges ≈ 1 M entries, trivial.
- **Quality:** DGR reports lower overflow and wirelength than CUGR-class routers at the global level (verified abstract claim).
- **Risk:** a limited pool means limited quality. Refresh the pool with a few FW steps under the current expected prices (column generation).
- **Ratings:** value 4, effort 2, GPU 5.

### I5. Entropic (soft-Bellman) multicommodity flow: stochastic user equilibrium
- **Refs:** Mensch & Blondel, differentiable DP (ICML 2018; verified). Akamatsu, "Cyclic flows, Markov process and stochastic traffic assignment" (TR-B 1996; verified). Fisk 1980 (unverified). Heat method for geodesics (Crane et al. 2013; unverified here).
- **Formulation:** V_n(x) = softmin_τ over neighbors y of (c(x,y) + V_n(y)). The soft flow of net n on an edge is ∝ exp(−(V_s→x + c + V_y→t)/τ), the forward-backward marginals, which are exactly the gradient ∂V_n(s)/∂c. The total objective Σ_n V_n^τ + Φ(U) is smooth and convex in the flows. That is entropy-regularized multicommodity flow, i.e. stochastic user equilibrium. Anneal τ → 0 to recover I3's hard solution.
- **GPU:** same scans as I1 in the log domain (logcumsumexp), with a forward and a backward field per net.
- **Why it helps:** smooth, unique solutions. It gives congestion maps for placement that are routing-aware rather than RUDY. Flows are directly samplable (I7).
- **Risk:** with cycles, the soft-Bellman fixed point exists only if exp(−c/τ) is contractive. Keep τ below the minimum edge cost and work in log space. Also, as τ gets small it converges slowly.
- **Ratings:** value 3–4, effort 3, GPU 5.

### I6. First-order LP (PDLP/cuPDLPx) on the tile-level multicommodity relaxation: a routability certificate
- **Refs:** cuPDLP.jl (Lu & Yang 2023, arXiv 2311.12180), cuPDLP-C (arXiv 2312.14832), cuPDLPx (arXiv 2507.14051, Halpern PDHG with PID primal weight). All verified.
- **Formulation:** minimize z (max overflow) or Σ slack, subject to per-net flow conservation (for Steiner nets: root-to-sink flows g_{n,t} ≤ x_n), Σ_n w_n·x_{n,e} ≤ cap_e + s_e, and x ≥ 0.
- **Why:** the matrix-vector products are stencils on the grid, so no sparse matrix is needed. You do not have to port PDLP; write a ~300-line WGSL PDHG kernel.
- **The distinctive output:** a **dual lower bound**. If the LP optimum shows overflow above zero at tile level, even with capacity slack for raster error, the placement is provably unroutable at that resolution, and you drop it before any fine work. Duals are also edge prices for I9.
- **Size:** about 10–30 M variables restricted to bounding boxes, ~1–5 k iterations, ~3 ms each, so 5–15 s per configuration. That is expensive for screening. Use it on the top 10% or at a 2 mm tile size.
- **Risk:** LP gaps for Steiner nets are weak. The bound is useful mainly for congestion, not for wirelength.
- **Ratings:** value 3, effort 3, GPU 5.

### I7. Randomized rounding by flow-following random walks (a diversity engine)
- **Refs:** Raghavan & Thompson, Combinatorica 1987 (unverified, standard).
- **Method:** given an acyclic fractional flow f_n (optimal with positive costs, or after cycle cancelling), a walk from the source that picks the next edge with probability ∝ outgoing flow has edge marginals exactly equal to f_n. One GPU thread per (net, sample) produces thousands of integral route sets per second. Correlated sampling (the same uniforms for nets in the same region) reduces overflow variance.
- **Use:** these samples are the corridor seeds for I2 fine routing, as an alternative or complement to I4.
- **Risk:** independent rounding creates local overflow, typically O(√(log n)·√cap). The fine negotiation absorbs it.
- **Ratings:** value 3, effort 1, GPU 5.

### I8. Perturbed and black-box differentiable routing for tuning and diversity
- **Refs:** Berthet et al. (NeurIPS 2020; verified). Vlastelica et al. (ICLR 2020, Dijkstra backpropagation; verified).
- **Use (a):** gradient of routed cost with respect to cost-map parameters. For example, learn per-board penalty weights such as via cost, layer bias and the history rate α, by averaging M perturbed shortest-path solves, which run in parallel.
- **Use (b):** Gaussian perturbation of the cost field is a principled "noise knob" for diversity. The expected solution is smooth in the noise scale ε.
- **Use (c):** gradients with respect to pin positions for joint place+route (I9) without a soft oracle.
- **Risk:** high variance, needing M ≈ 8–32 samples per gradient. That is cheap on the GPU but multiplies the cost.
- **Ratings:** value 2–3, effort 2, GPU 5.

### I9. Geodesic wirelength: joint place+route through distance fields (**top** candidate for "joint")
- **Idea:** replace HPWL/WA in the late ePlace stage with the soft Steiner cost of each net in the congestion- and obstacle-aware metric. Obstacles are other footprints' keepouts and fixed copper. Congestion prices come from I3/I5/I6 duals.
- **Gradient:** by the envelope theorem, ∂(net cost)/∂(pin position) = ∇V_n at the pin, bilinearly interpolated from the distance field. You get it for free from I1/I5, with no backpropagation through the solver.
- **Alternation:** every ~20 placement iterations, refresh the prices with a few FW steps.
- **Cost:** 200 nets × 60 k tiles ≈ 12 M cells per evaluation, ~5 ms.
- **Expected effect:** placements that route with fewer detours through connector/BGA bottlenecks. It captures blockage that RUDY misses completely, for example a net that has to go around a row of connectors.
- **Risk:** non-smoothness when the optimal route switches homotopy class. The soft τ helps. Keep WA wirelength mixed in as a regularizer.
- **Ratings:** value 4, effort 3, GPU 5.

### I10. Differentiable RUDY/probabilistic congestion plus cell inflation in ePlace (baseline)
- **Refs:** RePlAce, and routability DREAMPlace (Liu, Pu & Yu, DATE 2021, CNN congestion penalty; verified). RoutePlacer (KDD 2024; verified).
- **Method:** the RUDY map is a sum of per-net rectangles with value (w+h)/(w·h) per net, splatted via summed-area tables, so it is differentiable in the bounding box. Add a ∫ softplus(RUDY − cap)² penalty, or inflate components in hot areas.
- **Trade-off:** trivial on GPU and a known win, but weaker than I9 for PCBs.
- **Ratings:** value 3, effort 1, GPU 5.

### I11. Clearance-exact congestion via convolution linearity (enabler for I2)
- **Method:** keep a per-class occupancy occ_j (centerline and via cells). The conflict field for class k is C_k = Σ_j (occ_j ∗ D_{r_kj}), with r_kj = w_k/2 + w_j/2 + clr(k,j) + raster margin. Discs can be done as an FFT, or approximated by an octagon from 4 separable box passes via prefix sums. The own-net term is subtracted within the window, the same convolution restricted to it. Vias: splat across all layers with the via-pad radius (through vias).
- **Cost:** K² × 6 M × ~4 passes ≈ 10–20 ms per iteration at K = 4.
- **wgpu note:** WGSL has no float atomics. Splat into i32 fixed-point with atomicAdd, or use a gather formulation.
- **Ratings:** value 5, effort 2, GPU 5.

### I12. Top-2 nearest-net distance transform for raster DRC and "foreign copper distance"
- **Method:** jump flooding (Rong & Tan 2006; unverified) or separable EDT (Felzenszwalb & Huttenlocher; unverified), keeping the two nearest seeds with distinct net IDs. Each cell then knows the distance to the nearest copper of a different net. That gives an O(cells) raster DRC for all nets and classes at once, and an exact-ish "distance to violation" field for I14.
- **Cost:** ~log₂(1500) ≈ 11 passes × 6 M cells, ~10 ms.
- **Ratings:** value 4, effort 2, GPU 5.

### I13. Pours as connectivity-constrained regions: effective-resistance penalty plus GPU connected-component labelling
- **Formulation:** pour region P_n on layer l, with conductance σ(x) = σ₀·(1 − soft foreign occupancy dilated by the clearance). Pour pads are the terminals. Solve the Laplacian L(σ)φ = b (root = 1 A, pads = sinks) and penalize Σ R_eff(root, pad).
- **Gradient (adjoint, free):** ∂R/∂σ_e = −(φ_i − φ_j)². Where the current density is high is where cutting the pour hurts most. That becomes an extra cost term in the other nets' routing cost field, so traces are pushed away from pour bottlenecks and toward pour interiors that are redundant.
- **Solve:** multigrid or Jacobi-preconditioned CG at 0.25 mm, ~250 k cells per layer, a few ms.
- **Final:** actual pour fill = region minus foreign copper dilated by clearance. Then GPU connected-component labelling (e.g., Playne & Hawick 2018; unverified) to find islands. Islands that contain pour-net pads but are disconnected get stitching vias (candidates scored by the same φ) or are flagged. Islands without pads are removed or stitched according to the rules.
- **Risk:** R_eff is a proxy. Neck width is not strictly guaranteed, so a final min-width check is needed (morphological opening on the GPU).
- **Ratings:** value 4, effort 3, GPU 5.

### I14. Continuous post-route legalization: elastic polylines with signed-distance clearance (**top**)
- **Refs:** rubber-band/topological routing (SURF, Dai et al. 1991; unverified). Same spirit as ePlace, applied to wires.
- **Formulation:** simplify each routed raster path into a polyline with 45°/any-angle string pulling inside its own clearance corridor. Variables are the vertex positions, plus via positions. Minimize Σ length + β·bends + γ·Σ softplus(clr_{ij} + (w_i + w_j)/2 − dist(seg_i, seg_j))² + pad-attachment constraints. Topology is frozen, so no crossings can appear. dist(seg, seg) is closed form. Neighbor pairs come from a GPU spatial hash. The pad-to-pad topology stays fixed.
- **Solver:** Nesterov or L-BFGS, ~500 iterations × (≈20 k segments × ~20 neighbors) ≈ 0.2–0.5 s.
- **Why:** turns raster output into exact KiCad-style geometry with a positive clearance margin. It absorbs raster-quantization loss, which is what allows the 0.1–0.125 mm raster. Also makes clean 45° traces.
- **Risk:** stuck configurations where there really is not enough space. Detect them as residual violations and send them back to I2 with a local capacity decrement.
- **Ratings:** value 5, effort 3, GPU 4.

### I15. Continuous layer-assignment relaxation (2-layer boards especially)
- **Refs:** GAP-LA (arXiv 2507.13375, GPU layer assignment; verified that it exists). Classic Lagrangian relaxation for layer assignment.
- **Formulation:** each 2-D corridor segment gets a softmax over L layers. The expected via count is Σ over adjacent segments of (1 − p_a·p_b); per-layer congestion is the expectation. Anneal, then pick.
- **When it matters:** on 2–4 layers it is cheaper to route natively in 3-D (I1 already does). It is useful only if the tile stage is 2-D. Low priority.
- **Ratings:** value 2, effort 2, GPU 5.

### I16. Multi-pin topology via Gumbel-perturbed tree growth or a topology pool
- **Method:** Prim/Steiner growth in I1 order, with Gumbel noise on the attachment order, plus 2–4 alternative RSMT topologies (FLUTE-like lookup, or batched Iterated 1-Steiner on the GPU). Each variant goes into the I4 pool. For big nets (GND/power, which are mostly pours), route pad escapes only and let the pour connect (I13).
- **Ratings:** value 3, effort 2, GPU 4.

### I17. Beckmann/L1-flow PDHG at fine resolution: continuous "flow fields" per net
- **Refs:** Li, Ryu, Osher, Yin & Gangbo, "A Parallel Method for Earth Mover's Distance" (J. Sci. Comput. 2018; verified). This is the L1 min-cost flow m with div m = δ_s − δ_t, solved by PDHG with shrinkage. Everything is a stencil operation.
- **Multicommodity:** per-net vector fields m_n in the window. Coupling constraint Σ_n |m_n| ∗ D ≤ 1 (clearance through the convolution in I11). Solve with PDHG/ADMM; per-net shrinkage runs in parallel.
- **Assessment:** elegant, fully parallel, and it gives sub-cell "flow tubes". But memory is 2 floats per cell per net in the window (~160 MB for 20 M cells), and rounding a continuous flow into a width-w trace is nontrivial (extract the ridge, then I14). It is research-grade. Mainly interesting as a smooth fine-level initializer.
- **Ratings:** value 2–3, effort 4, GPU 5.

### I18. Portfolio and successive halving over random configurations (the meta-algorithm)
- **Method:** treat seeds (placement init, Gumbel/rounding seeds, cost weights, net-order noise) as arms. Budget allocation: 64–256 placements get tile-level I3 (~0.3–1 s each). The top 25% get I4 sampling with S = 4. The top 16 get fine I2. The top 4 get I13 + I14. The best gets CPU exact DRC.
- **Memory layout:** configurations are stacked along a batch dimension in one buffer so a single dispatch covers B configurations. That hides wgpu dispatch overhead, which matters more than FLOPs at this scale.
- **Ratings:** value 5, effort 1, GPU n/a.

### I19. GNN/CNN congestion or routability predictors (RouteNet-style), judged critically
- **Refs:** RouteNet (Xie et al., ICCAD 2018; unverified here). DREAMPlace-Cong (verified).
- **Assessment:** there is no public PCB routing dataset. The only realistic source is self-generated labels from your own router, which is about 1 k runs, feasible overnight. Even then it would at best approximate I3's tile-level output, which is already about 1 s on the GPU and exact for your cost model.
- **Where it could be worth it:** a tiny CNN predicting "fine-stage failure probability" from tile-level features, to prune in I18.
- **Ratings:** value 1–2, effort 3, GPU 5.

### I20. Diffusion or flow-matching generators (ChipDiffusion, DiffPlace, FlowPlace), judged critically
- **Refs:** ChipDiffusion (Lee et al., ICML 2025; verified). DiffPlace (arXiv 2510.15897; verified). CLDRoute (arXiv 2607.16674, routability map generation; seen in search, not read).
- **Assessment:** these are pretrained on large synthetic VLSI corpora and target macro placement. For PCBs you would need synthetic training generation plus guidance by exactly the differentiable objectives above, and the result would be a diverse initializer that ePlace immediately refines. Random ePlace inits already give diversity cheaply.
- **Routing via diffusion:** no exactness, no data. Not recommended.
- **Ratings:** value 1, effort 5, GPU 5.

### I21. Neural A* / learned heuristics (Yonetani et al., ICML 2021; verified)
- **Assessment:** the GPU scan oracle has no heuristic to learn. A* ordering is a serial concept. Only relevant for a residual CPU fallback. Skip.
- **Ratings:** value 1, effort 3, GPU —.

### I22. Coarse-to-fine multigrid routing: soft corridors instead of hard ones
- **Method:** prolong the tile-level dual prices and soft flows into the fine cost field as an additive prior: c_fine += λ·(−log p_corridor). Do not use a hard mask. This lets fine routing leave the corridor when that is locally better, fixing a classic failure of hard corridors like your current 16×16 tile corridors. Windows become corridor plus a 2–4 mm margin rather than the full bounding box.
- **Ratings:** value 4, effort 1, GPU 5.

---

## 2. Ratings summary

| # | Idea | Value | Effort | GPU share |
|---|---|---|---|---|
| I1 | Scan wavefront oracle (hard/soft) | 5 | 2 | 5 |
| I2 | Jacobi/ADMM PathFinder (proximal, random subset) | 5 | 3 | 5 |
| I3 | FW/Garg–Könemann tile MCF + column pool | 4 | 2 | 5 |
| I4 | DGR-style Gumbel selection over pool | 4 | 2 | 5 |
| I5 | Entropic soft-Bellman MCF (SUE) | 3–4 | 3 | 5 |
| I6 | PDHG LP certificate + duals | 3 | 3 | 5 |
| I7 | Flow-walk randomized rounding | 3 | 1 | 5 |
| I8 | Perturbed/black-box gradients | 2–3 | 2 | 5 |
| I9 | Geodesic WL (joint place+route) | 4 | 3 | 5 |
| I10 | Differentiable RUDY in ePlace | 3 | 1 | 5 |
| I11 | Convolution clearance congestion | 5 | 2 | 5 |
| I12 | Top-2 EDT raster DRC | 4 | 2 | 5 |
| I13 | Pour R_eff penalty + connected-component labelling + stitching | 4 | 3 | 5 |
| I14 | Continuous polyline legalization | 5 | 3 | 4 |
| I15 | Soft layer assignment | 2 | 2 | 5 |
| I16 | Gumbel Steiner topologies | 3 | 2 | 4 |
| I17 | Beckmann PDHG flow fields | 2–3 | 4 | 5 |
| I18 | Successive-halving portfolio | 5 | 1 | — |
| I19 | GNN/CNN predictors | 1–2 | 3 | 5 |
| I20 | Diffusion generators | 1 | 5 | 5 |
| I21 | Neural A* | 1 | 3 | — |
| I22 | Soft multigrid corridors | 4 | 1 | 5 |

## 3. Top 5
1. **I1 + I11 as one foundation:** the scan oracle plus convolution-based clearance congestion. Everything else depends on it, and on its own it moves the 20-minute kernel onto the GPU.
2. **I2:** Jacobi/ADMM PathFinder with a proximal term and randomized partial rerouting. This is the actual router.
3. **I14:** continuous legalization, with I12 as the raster check. It is the bridge to exact DRC-clean geometry and what makes the 0.1–0.125 mm raster possible.
4. **I3 + I4 (+ I7):** the tile-level FW column pool with Gumbel selection and flow-walk rounding. It is the source of diverse, globally balanced corridors and the screening signal for I18.
5. **I9 (with I13 feeding the cost field):** geodesic, price-aware wirelength in ePlace. This is the only piece that is genuinely joint place+route and cheap. I13 is included here because pours are otherwise the most common DRC-connectivity failure.

---

## 4. End-to-end GPU pipeline: 4-layer, 200-net, 150×100 mm board on a GTX 1650

Assumptions: fine raster 0.1 mm → 1500×1000×4 = 6 M cells; tile 1 mm → 60 k nodes; K = 3 classes; effective bandwidth ~90 GB/s; ~2× kernel inefficiency included. The numbers are order-of-magnitude estimates.

| Stage | What (GPU unless noted) | Batch | Est. time |
|---|---|---|---|
| 0 | Parse, net classes, static obstacle rasters per class (dilated keepouts via I12 EDT), on the CPU | 1 | 1–2 s |
| 1 | ePlace, B = 64 random inits in one batched dispatch: FFT density (256² × 64), WA wirelength + I10 RUDY. Late phase: I9 geodesic WL with prices refreshed every 20 iterations by 3 FW steps. Legalization of components (GPU or CPU) | 64 | ~1000 iters × ~4 ms ≈ 5–10 s |
| 2 | Tile-level screening: I3 FW/Garg–Könemann, 30 iterations, all nets, bounding-box-restricted, fp16 (~24 MB/config). Score = overflow + WL + vias. Optional I6 PDHG certificate for the top 16 | 64 | 64 × ~0.5 s ≈ 30 s (batched: ~15–30 s) |
| 3 | Top 16 placements: column pool (≤16 per segment) + I4 Gumbel selection (200 Adam steps), then S = 2 samples each via I4/I7 → 32 corridor sets | 16 | ~1 s each → 15 s |
| 4 | Fine routing, I2 over I1 + I11 + I22: windows ≈ 20 M cells. Per iteration ≈ 30% reroute (~3 GB traffic) + K² = 9 convolutions + history update ≈ 60–100 ms. ~40–60 iterations to zero raster overflow. About 200–300 MB per instance, so 4–8 instances concurrently | 32 | 32 × ~5 s ≈ 2–3 min (throughput-bound) |
| 4b | Pours: I13 R_eff at 0.25 mm folded into the Stage-4 cost every 10 iterations (≈3 ms each); final fill, connected-component labelling, stitching vias, min-width opening | 32 | ~0.3 s each |
| 5 | Rank; top 4 go to I14 continuous legalization (20 k segments, 500 iterations) + I12 raster DRC check | 4 | ~0.5–1 s each |
| 6 | CPU exact verifier (polygon clearance, spatial hash). Local fix-up: rip up offending nets and reroute them serially in small windows, or run I2 on the GPU restricted to the fix-up region | 1–3 | 2–10 s |
| **Total** | 64 placements screened, 32 fine-routed, best one DRC-verified | | **≈ 3–5 min** (vs ~20 min for one serial run). "Fast mode" (B = 4, S = 1): ~20–40 s |

- **GPU share:** ≈ 90–95% of arithmetic and memory traffic. Wall clock on the CPU is ~5–10% (parse, exact DRC, fix-up, orchestration).
- **At 0.05 mm raster:** Stage 4 costs about 4× more (~10–12 min). Recommend 0.1–0.125 mm + I14 instead.
- **Memory peak:** fine instances 8 × 250 MB + tile batch ~1.5 GB, within 4 GiB. Check the wgpu adapter limits `maxStorageBufferBindingSize` and `maxBufferSize`. The defaults are 128 MiB / 256 MiB and must be requested higher, or buffers chunked per layer or configuration. Use f16 via the `shader-f16` feature (Turing TU117 has fast FP16) and the subgroups feature for the scans.

### Cheap first experiments (PyTorch, CUDA or CPU)
1. **(1 day)** The I1 scan router: `cummin` / `logcumsumexp` sweeps with ±x, ±y, diagonals and vias. Check against current A* path costs on exported rasters.
2. **(2–3 days)** I2 + I11 on 3–5 real boards. Measure iterations to zero overflow as a function of reroute fraction and μ, plus wirelength and vias against serial PathFinder output.
3. **(2 days)** I3 tile FW. Correlate tile-level overflow and wirelength with final fine-route success over 50 random placements. This validates the screening funnel.
4. **(2 days)** I14 on the output of experiment 2. Measure the fraction of raster routes that become exact-DRC-clean without CPU fix-up.
5. **(1–2 days)** I13 R_eff penalty. Count pour islands and disconnected pads with and without it.

### Main risks overall
- Jacobi convergence on the last hard conflicts. Mitigation: the proximal term plus a CPU serial tail.
- Raster-to-exact margin loss. Mitigation: I14.
- wgpu ergonomics: no float atomics, dispatch overhead, buffer limits. Mitigation: fixed-point atomics and batched dispatches.
- Multi-pin/Steiner handling is heuristic in every relaxation.
- Differentiable methods (I4/I5/I9) add value mainly at the global and placement level. The fine level is best served by a fast exact oracle plus negotiation, not by a soft relaxation.

## References (verified = checked by web search in this session)
- DGR: Differentiable Global Router, DAC 2024 — https://dl.acm.org/doi/10.1145/3649329.3656530 (verified)
- GAMER: GPU Accelerated Maze Routing, ICCAD 2021 — https://dl.acm.org/doi/10.1109/ICCAD51958.2021.9643563 (verified)
- InstantGR, ICCAD 2024 — https://dl.acm.org/doi/10.1145/3676536.3676787 and https://github.com/cuhk-eda/InstantGR (verified that it exists; internals not read)
- FastGR, TCAD 2022 — https://dl.acm.org/doi/10.1109/TCAD.2022.3217668 (verified)
- GAP-LA — https://arxiv.org/abs/2507.13375v1 (verified)
- cuPDLP.jl — https://arxiv.org/abs/2311.12180; cuPDLP-C — https://arxiv.org/abs/2312.14832; cuPDLPx — https://arxiv.org/abs/2507.14051; GPU first-order LP overview — https://arxiv.org/abs/2506.02174 (verified)
- Mensch & Blondel, ICML 2018 — https://arxiv.org/abs/1802.03676 (verified)
- Berthet et al., NeurIPS 2020 — https://arxiv.org/pdf/2002.08676 (verified)
- Vlastelica et al., ICLR 2020 — https://arxiv.org/pdf/1912.02175 (verified)
- Akamatsu 1996, TR-B — https://www.sciencedirect.com/science/article/abs/pii/0191261596000033 (verified)
- Li, Ryu, Osher, Yin & Gangbo, J. Sci. Comput. 2018 — https://link.springer.com/article/10.1007/s10915-017-0529-1 (verified)
- Routability DREAMPlace, DATE 2021 — https://www.cse.cuhk.edu.hk/~byu/papers/C112-DATE2021-DREAMPlace-Cong.pdf (verified); RoutePlacer, KDD 2024 — https://dl.acm.org/doi/10.1145/3637528.3671895 (verified)
- ChipDiffusion, ICML 2025 — https://arxiv.org/abs/2407.12282 (verified); DiffPlace — https://arxiv.org/abs/2510.15897 (verified); CLDRoute — https://arxiv.org/pdf/2607.16674 (seen, not read)
- Neural A*, ICML 2021 — https://arxiv.org/abs/2009.07476 (verified)
- Corolla, GPU FPGA routing — https://ceca.pku.edu.cn/media/lw/137e5df7dec627f988e07d54ff222857.pdf (verified that it exists)
- Unverified, cited from memory: PathFinder (McMurchie & Ebeling 1995); Garg & Könemann (SIAM J. Comput. 2007); Raghavan & Thompson (1987); proximal Jacobian ADMM (Deng, Lai, Peng & Yin 2017); fast sweeping (Zhao 2005); heat method (Crane et al. 2013); jump flooding (Rong & Tan 2006); Felzenszwalb & Huttenlocher EDT; Playne & Hawick GPU connected-component labelling (2018); SURF rubber-band routing (Dai et al. 1991); RouteNet (Xie et al., ICCAD 2018); LeBlanc et al. FW traffic assignment (1975); Fisk (1980).
