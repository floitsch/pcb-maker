# Population-based GPU placement and routing for a KiCad placer/router: research memo

I did not read the repo. This memo covers ideas and literature only. References marked **[v]** I checked by web search in this session. References marked **[nv]** are well known but I did not re-check them.

## 0. Design principles, and why they matter on a GTX 1650

**The hardware.** TU117 has 14 SMs, 896 lanes, 4 GiB of memory at 128–192 GB/s, and 1 MB of L2. Shared memory is at most 64 KB per SM (Turing).

**wgpu constraints:**
- The default `maxComputeWorkgroupStorageSize` is 16 KB. Request the adapter limit, typically 48 KB on NVIDIA Vulkan.
- Only 32-bit atomics are portable.
- There is no grid-wide barrier.
- Each dispatch/submit costs roughly 10–50 µs.

These constraints decide the architecture.

1. **Split the work by kernel type.**
   - **Search kernels:** one workgroup handles one (replica, window/net) job and loops internally, using `workgroupBarrier` for BFS steps. Persistent workgroups pull jobs from an atomic counter. This avoids a global sync per wavefront step, which is what makes "one board across the GPU" maze routing (GAMER-style) awkward under wgpu.
   - **Evaluation kernels** (rasterize, DRC, CCL, RUDY): one board spread across the whole GPU, with the replica index as the z dimension of the dispatch.
2. **Keep candidates as vectors; rasterize only when needed.** A full-resolution per-replica raster costs too much:
   - 150×100 mm at 0.05 mm is 6 M cells/layer, so 24 M cells over 4 layers. An owner id (u16) is 48 MB per replica, so fewer than ~80 replicas fit.
   - At 0.1 mm it is 12 MB per replica, fewer than ~300.
   - A routing encoded as polylines (i16 x, i16 y, u8 layer, u8 flags: 6 B per vertex; ~20 vertices × ~2.5 connections × 200 nets) is about 60 KB. That allows ~50k candidates in 4 GiB.
   - Rasterize windows on demand from the vectors. For full-board replicas you really need, use copy-on-write tiles (64×64-cell tiles; each replica keeps a tile table pointing into a shared pool and stores only dirty tiles).
3. **Static data is shared by all replicas:** pads, keepouts, board edge, and per-net-class inflated obstacle bitplanes. At 0.1 mm a bitplane is 1.5 M bits = 190 KB per layer, so 5 classes × 4 layers ≈ 4 MB.
4. **Compute clearance in configuration space.** A net's centerline may enter a cell only if foreign copper is at least w_n/2 + clr(n, ·) away.
   - In a window this is a dilation of the foreign-copper bitplane by an octagon that encloses the disk. Alternate 4- and 8-neighbour shift-ORs, taking r steps. Enclosing the disk is conservative.
   - After routing, paint the net's copper as dilate(path, w_n/2).
   - A via is legal where the via-radius configuration space is free on all layers it spans. Through vias need one AND-ed plane per window.
   - Keep a 1-cell safety margin so the final exact CPU DRC rarely has anything to fix.
5. **The core search kernel is a bit-parallel Lee wavefront** in workgroup memory. Each u32 holds 32 cells, and one step is:

   ```
   F' = (F | F<<1 | F>>1 | Fup | Fdown | viaMove(F)) & ~Obs & ~Vis
   ```

   Horizontal shifts need cross-word carries.
   - **Memory:** a window of 128×128×4 is 64 Kbit = 8 KB per plane. Obs + Vis + Frontier = 24 KB of workgroup memory, which fits 2 workgroups per SM. The 2-bit "step mod 3" backtrace labels (16 KB per job) go to global scratch, where they stay in L2.
   - **Throughput estimate:** about 2048 words × ~15 ops per step is ~0.3–0.5 µs per step per workgroup. A 200-step route is then ~100 µs, and with ~28 workgroups resident the rough ceiling is 10⁵ window routes/s. That is 1–2 orders of magnitude above CPU A* on the same window. This is an estimate; measure it.
   - **Non-uniform costs: "bit-sliced Dial".** A cell of cost c enters a chain of c−1 delay planes before it joins Vis. With 3–4 cost levels (free, present-congestion, history-high, forbidden) the whole thing stays bitwise. Via cost is a delay chain of length V on the layer transition.
   - **Anything finer (bend costs, exact history weights)** can be sampled as random blocking masks. A cell with cost h is blocked with probability p(h). Each replica sees a different mask, so you get diversity for free (perturb-and-MAP; Papandreou & Yuille, ICCV 2011 **[v]**).
   - **Multi-pin nets:** run a multi-source BFS from the whole current tree to the nearest unconnected pin, and repeat. This is Prim-Steiner and costs nothing extra in Lee. Randomize tree topology per replica.
   - **Path shape:** 8-connected BFS gives Chebyshev-metric paths. Do a GPU/CPU string-pull/45° smoothing pass afterwards.
   - **Replica packing:** instead of packing 32 cells per word you can pack 32 replicas of the same window, one bit each. This is the asynchronous-multispin-coding trick from spin-glass codes (Fang et al., CPC 185, 2014 **[v]**). It is useful for LNS, where 32 random restarts attack the same window.

## 1. The ideas (22)

### A. Evaluation infrastructure (everything else uses it)

**1. Batched GPU evaluator.**
- **Per-replica copper rasterization:** capsules and disks scatter into a u32 cell using packed `atomicMin`/`atomicMax` of net id. A cell with min ≠ max is a short or clearance violation (after inflation).
- **Per-net connectivity:** run CCL on the owner raster restricted to each net's cells, and require exactly 1 component containing all pads. Use Playne–Hawick equivalence (TPDS 2018 **[v]**, CUDA reference code on GitHub **[v]**), Komura equivalence (CPC 2015 **[v]**), or block-based BKE (Allegretti et al., TPDS **[v]**). Label propagation is simpler, but its iteration count is O(diameter), which is bad for long snakes.
- **Pours:** free = pour region − dilate(foreign copper, clr). Do a 3D CCL across layers joined by stitching vias. The metrics are the number of islands that contain pads of the pour net, dead islands, and minimum neck width (from an EDT: Felzenszwalb–Huttenlocher separable EDT **[nv]**, or jump flooding, Rong & Tan 2006 **[nv]**).
- **Congestion:** RUDY (Spindler & Johannes, DATE 2007 **[v]**) using a 4-corner scatter-add followed by a 2D prefix sum, which is O(nets + cells).
- **Placement:** courtyard overlap via a 0.25 mm atomic-count raster.
- **Cost:** a full 0.1 mm, 4-layer evaluation is about 3–4 passes over 3 M cells, roughly 50 MB of traffic, about 0.5 ms. So about 2000 full evaluations/s; coarse screening is 10× cheaper.
- **GPU share:** 100%.

**2. Exact-geometry GPU DRC on vectors.** Build a spatial hash of segments and arcs and test every pair in the same bucket. At ~3k pads and ~20k segments that is trivial. Add a GPU elastic-band "shove" repair: move vertices along clearance-violation normals, with several replicas using different step policies. The final CPU exact verifier becomes a formality.

### B. Placement populations

**3. Batched ePlace, one replica per workgroup.**
- With 250 components and a 64×64 density grid in f32 (16 KB), the DCT and Poisson solve fit in workgroup memory.
- About 0.5 MFLOP per Nesterov iteration × 300 iterations × 1024 replicas is ~150 GFLOP, about 0.5–1 s.
- Replicas differ by seed, net weights, target density, initial orientation/side assignment, and fixed-group constraints.
- This is DREAMPlace's kernel set (Lin et al., DAC 2019 **[v]**) turned from "one huge design" into "thousands of tiny designs".
- A replica is ~16 B per component (x, y, θ, side), so ~4 KB. Memory is irrelevant.

**4. GPU parallel tempering for discrete placement moves** (swap, rotate 90°, flip side, align groups, legalize).
- One replica per workgroup, with incremental HPWL over the ≤10 affected nets and a courtyard bitmask in shared memory.
- Use a non-reversible PT ladder (Syed et al., JRSS-B 2022 **[v]**) with 16–32 temperatures × 32–64 independent ladders.
- This replaces the CPU SA legalizer. Moves are local, so use checkerboard/independent-set parallelism *within* a replica only if you need it.

**5. Successive halving / racing across fidelities** (the key mechanism for "quantity beats quality").
- 4096 placements → RUDY → keep 1024 → 1 iteration of GPU global routing → keep 128 → 5 negotiated GR iterations → keep 8 → detailed routing.
- Budget per rung is constant: Hyperband-style (Li et al., JMLR 2018 **[nv]**).
- This is cheap, and it is exactly where independent random runs do pay off, because placement quality is a fat-tailed determinant of routability.

**6. CMA-ES / ES on low-dimensional knobs.** Tune ePlace weights, target density, router cost parameters (via cost, present/history factors, their growth), and cost-noise scale, with GPU fitness.
- sep-CMA or LM-MA-ES could also act directly on the ~750 coordinates, but ePlace gradients beat that. Use ES only for the non-differentiable routability signal, e.g. ES gradient of GR overflow added to the ePlace gradient.

### C. Global and corridor routing populations

**7. GPU global routing with thousands of replicas.**
- Use 1 mm gcells: 150×100×4 = 60k nodes. Per-replica edge usage is u8 × (2 planar + 1 via) ≈ 180 KB, so ~2–4k replicas can live on the device.
- Each net search is a bbox BFS (bit-sliced Dial, or GAMER-style sweeps, Lin/Wong TCAD 2023 **[v]**; see also "Superfast full-scale GPU-accelerated global routing", ICCAD 2022, and InstantGR **[v]** for batching nets whose 3D bboxes don't overlap).
- Pin-access and escape capacity per gcell come from a static precomputation (BGA channels!).

**8. Population PathFinder with shared history (ACO-PathFinder).** This is the most important routing idea.
- Independent replicas throw away negotiation's key asset, the learned history cost.
- Instead, R replicas route concurrently with costs = base + present(own replica) + **shared** history + per-replica noise and random net order.
- After each epoch, history += f(overuse aggregated over replicas), optionally elite-weighted. That is ant-colony pheromone applied to PathFinder, and it keeps convergence behaviour while parallelizing across replicas × non-overlapping nets.
- It works both at GR level and at corridor-detailed level.

**9. Estimation-of-distribution / cross-entropy over routing "genes".**
- Genes: per-net priority (Plackett–Luce over orders), preferred layer pair, detour budget, Steiner topology choice, homotopy side at hotspots.
- Sample 256–1024 replicas, keep the elite 10%, refit.
- This is the cheapest form of the "learned proposal distribution trained on the fly". It needs no neural network, the state is a few floats per net, and it converges in 5–20 generations.

**10. Topological (homotopy) candidate encoding.**
- Triangulate pads and obstacles (CPU, once). A net's route is its crossing sequence through triangulation edges, a few bytes.
- Feasibility check: for every triangulation edge, Σ(width + clearance) of crossing nets ≤ edge length (cut capacity), plus via budget. That is O(edges) per replica, so **millions** of candidates per second.
- Only the survivors get geometric realization (rubber-band + spacing, or raster routing constrained to the corridor).
- Topological/rubber-band PCB routers: SURF, Dayan's thesis, TopoR **[nv; I could not verify the exact citations]**.
- It is the right abstraction for PCB because any-angle routing and hugging are natural there. The cost is new infrastructure (triangulation, rubber-band realization).

### D. Detailed routing and repair

**11. Parallel windowed LNS with non-overlapping commit.** This is the workhorse.
- Per conflict or hotspot, cut a window (96–128 cells square × 4 layers at 0.05–0.1 mm). Rip up every net inside it and pin the boundary crossings of nets that pass through.
- Launch K = 32–256 restarts per window (random order, random masks, random via budget, one workgroup each). Keep the best.
- Then commit a maximal independent set of non-overlapping improved windows into the master (greedy colouring on GPU), and repeat.
- Why quantity wins here:
  - Windows are small, so per-restart success probability is decent and run-time tails are cut by restarts (Gomes, Selman, Crato & Kautz, JAR 24, 2000 **[v]**).
  - Decomposition turns best-of-N into best-of-N **per region**, see §3.

**12. Net-level partition crossover (the GPX analogue).**
- For parents A and B, take D = the set of nets whose routes differ.
- Build a graph on D: nets i and j are linked if route_i^A conflicts (after clearance) with route_j^B or vice versa, or if they share a pour neck.
- Connected components of this graph can be chosen independently, so pick the better parent per component. On any objective that decomposes over components, the child is at least as good as both parents, exactly like GPX tunnelling between local optima (Tinós, Whitley & Ochoa, Evol. Comput. 2020 **[v]**; a GPU GPX paper, arXiv 2608.21233, exists **[v]**).
- Computing it: rasterize A and B, then run union-find over net ids (one kernel pass plus a small CPU/GPU union-find).
- Multi-parent version: choosing per component from M parents gives a small set-packing problem, which leads into idea 13.

**13. Path-selection QUBO with column generation, solved by GPU PT/SA.**
- Variables: x_{c,k} = connection c uses candidate path k, where the K = 8–32 candidates come from randomized router runs, crossover parents, and the LNS archive.
- H = A·Σ_c(1 − Σ_k x_{c,k})² + B·Σ conflict(c,k; c',k')·x·x' + Σ len·x.
- Pricing step: re-route unsatisfied connections under dual/penalty costs from the current best, add columns, re-solve.
- Sizes: 400 nets × 2.5 × 16 ≈ 16k spins. J is local and sparse, ~10⁶ nonzeros in CSR (~10 MB, shared). Replica state is 2 KB (bits) or 32 KB (fp16 for SB), so thousands of replicas.
- Solver: sparse J favours Metropolis PT with a colouring of the conflict graph over simulated bifurcation. SB (Goto et al., Sci. Adv. 2019 **[v]**, and the ballistic/discrete SB follow-up, Sci. Adv. 2021 **[v]**) shines on dense couplings. Fujitsu's Digital Annealer parallel-trial plus dynamic offset (Aramon et al., Front. Phys. 2019 **[v]**) is a good single-replica escape scheme.
- The same machinery handles **layer assignment** (spins = layer per 2D segment, couplings = via cost + same-layer conflicts).
- Limitation: path selection can only recombine what the generator produced. It is a combiner, not a router.

**14. Window SAT/MaxSAT for the last few hotspots.**
- Use a one-hot path encoding over enumerated candidate paths, as in idea 13. Cell × net encodings with connectivity constraints are hostile to local search.
- GPU WalkSAT/probSAT with thousands of independent chains per window is plausible.
- ParaFROST (Osama & Wijs, TACAS 2021 **[v]**) is CDCL with *GPU inprocessing*, not a local-search solver, so it doesn't fit this role directly.
- Realistically a CPU kissat call on ≤10-net windows is the exactness backstop, and it also gives an *unroutability proof* to report back to placement.

**15. Competing-wave cellular automaton ("territory growth").**
- All nets' wavefronts expand simultaneously; the first wave to arrive claims a cell (with clearance halo). Each net then routes inside its own territory.
- Non-overlap is guaranteed by construction. Many random speed/priority fields give a cheap, highly diverse initializer.
- Quality is weak alone. Use it as seed diversity for 8/11/12.

**16. Multi-commodity Physarum / current-flow relaxation.**
- Per net, Jacobi-solve Kirchhoff potentials on its bbox; conductance update D ← f(|flow|) − competition(other nets' flow); iterate until tube-like paths emerge (Tero et al., Science 2010 **[nv]**).
- It is purely data-parallel stencil work, 100% GPU. It gives soft congestion maps and good topologies in dense regions.
- Risk: slow convergence and fragile multi-commodity behaviour. Research-grade.

**17. Batched rollouts / root-parallel MCTS over net ordering and homotopy decisions at a hotspot.**
- Rollout = the batched greedy router finishing the window.
- Rollouts are ~0.1–1 ms of workgroup time, so tree size is modest. CEM (idea 9) usually dominates MCTS unless decisions are strongly sequential (escape order under a BGA).
- Use MCTS only for BGA/connector fan-out, where order is everything.

**18. Quality-diversity archive (MAP-Elites, Mouret & Clune 2015 [nv]).**
- Descriptors: via count, layer-usage ratio, homotopy signature at the top-k hotspots, pour island count.
- The archive feeds parents to crossover (12) and columns to the QUBO (13).
- It directly counters the "all replicas fail the same way" failure mode by forcing different homotopies to survive.

**19. Pour-aware routing cost ("do not cut the pour").**
- Per replica, compute neck criticality of each pour layer from an EDT plus a CCL of the pour-free region (cells whose occupation would split an island or drop the neck below min width).
- Use it as a cost level in the bit-sliced Dial router, and put stitching-via placement into the population (PT over via positions, fitness = pour island count + current-path proxy).

**20. Online learned proposal (small CNN/U-Net).**
- Input: placement and pad rasters plus the current replica's congestion.
- Output: per-cell conflict probability (RouteNet-style), trained online on replica outcomes, which you get by the thousand for free.
- Use it to shape noise and masks, and to pick windows for LNS. On a 1650 a 64×64 → 64×64 U-Net trains in seconds.
- Don't do this before 8/11/12 work. It is second-order.

**21. Adaptive restarts and early kill.**
- Luby-style budgets per replica. Kill replicas whose overflow trajectory falls below the population quantile after t epochs, and respawn them from elites plus mutation. This is PBT, population-based training, applied to routers.
- Keep ~20% of slots reserved for fresh random replicas to preserve diversity.

**22. Correlated-failure detector with escalation.**
- If > X% of replicas have unresolved conflicts inside the same small region, the problem is structural, not stochastic.
- Escalation order: (a) local placement-repair population (PT moving the 3–10 components around the hotspot, re-evaluated with GR); (b) window SAT proof; (c) report "needs another layer / a different footprint".

## 2. Do many weak runs beat one strong run?

- **Independent best-of-N gives weak returns.** For roughly Gaussian final scores, E[best of N] − mean ≈ σ·√(2 ln N): 3.0σ at N = 100 and 3.7σ at N = 1000. That is diminishing. A strong serial negotiated router typically sits several σ above a random-order single pass, so **naive "1000 random PathFinders, keep the best" will often lose to one good negotiated run** on dense boards.
- **Decomposition changes the math.** If the board splits into R roughly independent regions and you select per region (LNS commit, net-level crossover, QUBO path selection), the gain is Σ_r σ_r·√(2 ln N) with σ_r ≈ σ/√R. That is √R times the monolithic gain, and it searches N^R combinations. **Quantity only pays when paired with recombination or decomposition.** This is the core design rule.
- **Heavy tails.** Randomized sequential routers/solvers often have heavy-tailed time-to-legal distributions (documented for backtracking search by Gomes et al.; not established for PathFinder, so measure it). Where tails are heavy, many short runs plus restarts give super-linear speedups. That is the natural regime for small windows (idea 11), not whole boards.
- **Correlated failure is the main risk.** On dense boards infeasibility is often structural: a BGA escape capacity or connector fan-out impossible under the current placement. Every replica then fails at the same cells, and N doesn't matter. The mitigations:
  - share learning (shared history, idea 8; CEM, idea 9);
  - diversify at structural levels (placement, topology, layer assignment), not just net order;
  - keep a QD archive;
  - detect the situation and escalate to placement (idea 22).
- **Where independent quantity clearly wins:** placement screening (idea 5), because routability is very sensitive to placement and cheap GR fitness makes N = 1000s affordable, and window restarts (idea 11).

## 3. Top 5

1. **ACO-PathFinder (8), built on the batched bit-sliced Dial window router (§0.5) and the GPU evaluator (1).** It keeps negotiation's convergence and gets R× parallelism, both at GR level (thousands of replicas) and at corridor detail level (tens of replicas).
2. **Parallel windowed LNS with independent-set commit (11).** This is where the owner's "quantity beats quality" insight actually holds: small windows, heavy tails cut by restarts, and per-region selection.
3. **Net-level partition crossover (12), plus multi-parent QUBO path selection (13) for the residue.** These turn a population into one solution better than any member, cheaply.
4. **Placement population + successive halving on GPU GR fitness (3, 4, 5, 7).** This is the biggest lever on routability, it is embarrassingly parallel, and it moves the ePlace/SA work fully onto the GPU.
5. **Topological crossing-sequence encoding (10)** as the long-term candidate representation, with millions of O(edges) feasibility checks per second. Near term, the QD archive (18) gives much of the diversity benefit at far lower cost.

## 4. End-to-end "population on GPU" pipeline

Board: 4 layers, 200 nets, ~150 components, ~2000 pads, 100×80 mm, 0.1 mm detail raster (1000×800×4 = 3.2 M cells). All numbers below are order-of-magnitude estimates to be measured. "WG" means workgroup-time; 28 concurrent WGs are assumed.

| Stage | Candidates | Kernel work | GPU time |
|---|---|---|---|
| P1 batched ePlace | 1024 replicas, 1 per WG | 300 Nesterov iterations; WA wirelength grad + density scatter + 64² DCT | ~0.5–1 s |
| P2 PT legalize/refine | 1024 (32 ladders × 32 T) | ~20k moves/replica, ~2 µs WG each | ~1 s |
| P3 screen | 1024 → 128 | RUDY (µs each) + 1 GR pass, 1 mm gcells (200 nets × ~20 µs) | ~0.2 s |
| P4 GR race | 128 → 8 | 5 negotiated GR epochs | ~0.3 s |
| R1 ACO-GR | 8 placements × 128 replicas | 10 epochs × 200 nets, shared history per placement: 2 M net-routes × ~20 µs / 28 | ~1.5 s |
| R2 corridor detail | 8 × 8 replicas (COW tiles) | bit-sliced Dial in GR corridors (≤128×128×4 windows, ~100 µs per net), ~2.5 passes: 32k routes | ~0.15 s + eval 64 × 20 × 0.5 ms ≈ 0.6 s |
| R3 LNS | 16 best solutions | per iteration 32 hotspot windows × 32 restarts × ~8 nets × 100 µs; 10 iterations | ~3–5 s |
| R4 crossover + QUBO | 8 parents per placement → 1 | union-find over nets; hotspot QUBO ~2k spins × 1024 replicas × 5k sweeps | ~0.3 s |
| R5 pours/stitching | top 4 | 3D CCL + EDT + PT over via positions | ~0.2 s |
| Final | 1 | vectorize, string-pull, GPU vector DRC + elastic shove; CPU exact KiCad-rule verify | ~0.5–1 s (mostly CPU) |

**Totals.** About 8–12 s wall-clock, against today's seconds to 20 min. Over 90% of arithmetic/memory work is on the GPU. The CPU keeps orchestration, the triangulation (if idea 10 is used), exact verification and SAT fallbacks.

**Memory:**

| Item | Size |
|---|---|
| Static planes | ~5 MB |
| GR replicas | 1024 × 180 KB ≈ 180 MB |
| 64 detail replicas via COW | < 400 MB |
| Vector archive | 10k × 60 KB = 600 MB |
| LNS scratch | ~10 MB |
| **Total** | **well under 2 GiB** |

## 5. Cheap first experiment (days, not weeks)

**Step 1, no GPU: test the premise on the existing CPU router.**
- On 5 boards (sparse → dense), run the current PathFinder with N = 100 seeds (random net order, ±20% cost noise).
- Log the time-to-legal distribution (is it heavy-tailed?), final wirelength/vias, and the per-region conflict heatmap.
- Offline, compute the **oracle per-window combination**: tile the board into 10 mm cells and, for each cell, pick the replica with the fewest conflicts/lowest cost among those consistent on boundary nets, using the net-level crossover rule of idea 12.
- If the oracle combination beats the single best run by a clear margin, and conflicts are *not* concentrated in the same cells across ≥80% of replicas, the population approach is justified. If failures are correlated, the priority shifts to placement populations.

**Step 2, one wgpu kernel.**
- Implement the bit-parallel Lee/Dial window router: 128×128×4, 24 KB workgroup memory, persistent workgroups, 3 cost levels, via delay chain.
- Benchmark window routes/s on the 1650 against the CPU A* on identical windows cut from real boards.
- Go/no-go threshold: ≥20× the throughput of one CPU core on the same windows. That makes ideas 8 and 11 viable as the main path.

## Sources
- GAMER: https://ieeexplore.ieee.org/document/9799536/ ; https://dl.acm.org/doi/10.1109/ICCAD51958.2021.9643563
- Superfast GPU global routing: https://dl.acm.org/doi/pdf/10.1145/3508352.3549474 ; InstantGR: https://www.researchgate.net/publication/390645719_InstantGR_Scalable_GPU_Parallelization_for_Global_Routing
- Corolla (GPU FPGA routing, Bellman-Ford in PathFinder): https://ceca.pku.edu.cn/media/lw/137e5df7dec627f988e07d54ff222857.pdf
- Playne–Hawick CCL: https://github.com/DanielPlayne/playne-equivalence-algorithm ; Allegretti et al. BKE: https://iris.unimore.it/bitstream/11380/1179616/1/2018_TPDS_Optimized_Block_Based_Algorithms_to_Label_Connected_Components_on_GPUs.pdf ; Komura (via https://arxiv.org/abs/1603.08357)
- Simulated bifurcation: https://www.science.org/doi/10.1126/sciadv.aav2372 ; https://www.science.org/doi/10.1126/sciadv.abe7953
- Digital Annealer: https://arxiv.org/abs/1806.08815
- GPU PT spin glass (CAMSC): https://arxiv.org/abs/1311.5582 ; Non-reversible PT: https://rss.onlinelibrary.wiley.com/doi/10.1111/rssb.12464
- Near-Far SSSP: https://mgarland.org/papers/2014/sssp/ ; GPU A*: https://cdn.aaai.org/ojs/9367/9367-13-12895-1-2-20201228.pdf
- GPX: https://direct.mit.edu/evco/article/28/2/255/94983/ ; GPU GPX: https://arxiv.org/abs/2608.21233
- Perturb-and-MAP: https://dl.acm.org/doi/10.1109/ICCV.2011.6126242
- ParaFROST: https://link.springer.com/chapter/10.1007/978-3-030-72016-2_8
- Heavy tails: https://link.springer.com/article/10.1023/A:1006314320276
- RUDY: https://ieeexplore.ieee.org/document/4211973/
- DREAMPlace: https://dl.acm.org/doi/10.1145/3316781.3317803
- Multi-agent route QUBO (path-selection form): https://arxiv.org/html/2602.07913
