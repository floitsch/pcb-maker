# GPU-parallel PCB routing: parallel shortest paths, wavefronts and GPU routers

Legend: **[W]** means I checked the reference by web search in this session (title, venue and main claim). **[M]** means it is well known but I did not check it here. Numbers without a citation are my own engineering estimates, typically good to within 2–3×.

## 0. Framing

- **Where the work is.** Serial A* on the lattice expands roughly 10–20% of a net's window. A GPU wavefront touches its whole search region several times. So the GPU only wins if (a) the search region is kept small (corridor plus pruning), (b) many searches share one dispatch, and (c) almost no CPU↔GPU synchronisation happens in the inner loop.
- **Where parallelism should come from.** Your data point is that Jacobi over all nets gave 448 s / 44 vias against 81 s / 28 vias serial. That says intra-replica parallelism across nets is expensive in quality. Independent replicas cost nothing in quality. So parallelism should come mainly from **R replicas × small Gauss–Seidel batches (B ≈ 8–16)**, not from routing all nets at once.
- **wgpu constraints that shape the design:**
  - Only 32-bit atomics (`atomicAdd`, `atomicMin`, `atomicMax` on u32) are portable.
  - Default workgroup storage is 16 KiB.
  - Default `maxStorageBufferBindingSize` is 128 MiB; request the adapter limit, which is usually ≥2 GiB on Vulkan desktop.
  - There is no safe cross-workgroup global barrier, so avoid persistent kernels that spin.
  - `dispatch_workgroups_indirect` is available. Use it with a small GPU-side scheduler kernel so each outer step needs no CPU readback, and check termination only every ~32 steps.

---

## 1. Ideas

### Idea 1: Tile-asynchronous Bellman–Ford in corridor windows (core kernel)
- **Refs:** Teodoro et al., *Efficient Irregular Wavefront Propagation Algorithms on Hybrid CPU-GPU Machines*, arXiv 1209.3314 [W]; Davidson, Baxter, Garland, Owens, *Work-Efficient Parallel GPU Methods for SSSP*, IPDPS 2014 (Workfront Sweep / Near-Far / Bucketing) [W]; Corolla (Shen & Luo, FPGA 2017, search limited to subgraphs that expand dynamically) [W].
- **GPU mapping:**
  - Unit of work = a tile of 16×16 planar nodes × all L layers, i.e. 1024 nodes at L=4. Your coarse 16×16 tile graph already uses this size, so a corridor is exactly a set of tiles.
  - Buffers:
    - `dist` arena (u32): 26 bits of fixed-point cost plus 4 bits of parent direction (8 planar directions, up, down), one slab per active search.
    - `tile_state`: per search, per tile, an active flag and the minimum boundary distance.
    - `work_list`: (search_id, tile_id) pairs.
    - Global cost inputs: see Idea 6.
  - Kernel A, `relax_tile`, one workgroup per work-list entry:
    - Load the tile plus a 1-node halo of `dist` into shared memory (18×18×4×4 B ≈ 5.2 KB).
    - Compute per-node step costs on the fly from the occupancy, history and static fields.
    - Iterate to local convergence using directional row, column and diagonal sweeps (Idea 2) inside shared memory; usually 2–4 rounds.
    - Write back. If a boundary node improved, `atomicMin` the neighbour tile's `min_boundary` and set its active flag.
  - Kernel B, `schedule`: compact the active tiles into `work_list` and write the indirect-dispatch arguments. Optionally keep only tiles with `min_boundary < frontier_min + Δ` (Idea 3).
  - Each outer step is 2 dispatches. The number of steps per search is about 1.5–3 × the path length in tiles.
- **Work per search:** about 150 tile-visits for a 25 mm connection in a corridor 3 tiles wide, i.e. ~150 × 1024 nodes × ~60 relaxations per node ≈ 9 M relaxations and ~3 MB of traffic.
- **Widths/clearance:** folded into the step cost per net class (Idea 6).
- **Vias:** the layer dimension lives inside the tile, so via relaxation is a per-(x,y) min-plus pass across the 4 layers in shared memory. Via cost is `via_cost + conflict(via footprint)`.
- **Multi-pin:** seed `dist = 0` on all nodes of the current tree (multi-source). Targets are all unconnected pins. Or use Idea 8.
- **Pours:** own-net pours are sources/targets; other nets' pours are static obstacles.
- **Memory:** corridor of ~70 tiles × 4 KB ≈ 0.3 MB per search, so 256 concurrent searches take ~80 MB.
- **Quality:** exact shortest paths within the corridor, equivalent to A* restricted to the corridor. Corridor too narrow means failure; widen it and retry, as in Corolla's dynamic expansion.
- **Replicas:** a search id encodes (replica, net), so one dispatch serves all replicas.
- **Risk:** spiral or maze-like obstacles re-activate tiles many times. Pathological cases fall back to CPU A*.
- **First experiment:** implement it in wgpu for a single 2-pin net on a static cost map. Compare path cost against the CPU A* (should be equal up to tie-breaking), then time 1, 16 and 128 simultaneous searches.
- **Ratings:** value **H**, effort **M–H**, GPU fraction of search work ≈ 100%.

### Idea 2: GAMER-style alternating directional sweeps as min-plus scans
- **Refs:** Lin, Liu, Young, Wong, *GAMER: GPU-Accelerated Maze Routing*, TCAD 42(2) 2023 [W]. It splits shortest-path search into alternating horizontal and vertical sweeps, takes a sweep from O(n²) to O(log n), and reports 2.7× overall inside CUGR. Also the fast sweeping method (Zhao, Math. Comp. 2005) [M].
- **Key trick:** along a line, `d_i ← min(d_i, d_{i-1} + w_i)` is a composition of maps `f(x) = min(a, x + w)`, which are closed under composition: `(a2,w2)∘(a1,w1) = (min(a2, a1+w2), w1+w2)`. That makes each directional sweep a parallel prefix scan over (a, w) pairs.
- **Mapping:** eight directional scans (E, W, N, S and four diagonals, with √2 fixed-point diagonal costs) plus an up/down layer scan make one round. Repeat until no value changes (global flag, read back every few rounds). The number of rounds is roughly the number of direction changes the optimal path needs, plus one.
- **Use:**
  - Inside Idea 1's tiles, as the local solver (32-wide scans in shared memory).
  - Standalone for wide windows and at the coarse level. Each directional sweep touches the whole window once: about 12 B per node per sweep, ~100 B per node per round.
- **Quality:** exact at convergence. Bend penalties need direction-augmented state (Idea 22).
- **Risk:** the round count explodes around U-shaped obstacles when the window is large, which is why it belongs inside tiles.
- **Ratings:** value **H** as the in-tile solver, **M** standalone; effort **M**; GPU ≈ 100%.

### Idea 3: Delta-stepping at tile granularity (tile-level near-far)
- **Refs:** Meyer & Sanders, delta-stepping, J. Algorithms 2003 [M]; Davidson 2014 Near-Far [W]; ADDS (Wang et al., PPoPP 2021: asynchronous, Δ chosen dynamically, approximate priority queue, 2.9× over prior GPU SSSP) [W]; MultiQueue-based FPGA routing with relaxed A* priority (FPT 2024) [W, title only].
- **Mapping:** `schedule` keeps a per-search frontier minimum. It only emits tiles with `min_boundary + h(tile) ≤ frontier + Δ`; the other tiles stay in a far pile. Choose Δ at about 1–2 tile-widths of base cost. Adding h makes this an A*-ordered tile schedule.
- **Gain:** cuts redundant tile revisits from 3–5× to about 1.3–2×, at the cost of more outer steps. Dispatch overhead is amortised over the batch and the replicas.
- **Risk:** more steps per search raises latency, which hurts Gauss–Seidel batch turnaround. Tune Δ.
- **Ratings:** value **M–H**, effort **L–M** once Idea 1 exists, GPU 100%.

### Idea 4: Bounded, A*-pruned wavefront (ellipse pruning)
- **Refs:** standard A* bounding [M]. Corolla and Shen 2019 use a similar subgraph-limitation idea [W].
- **Mapping:**
  - Upper bound UB = the cost of the net's previous route under the current prices. It is a valid path, so this is always a valid bound.
  - Per-node lower bound h = octile distance × minimum cost per step.
  - Mask any node with `d + h > UB·(1+ε)`, and skip a tile when `min_boundary + h_min(tile) > UB`.
  - Intersect with the corridor from Idea 9.
  - First iteration: take UB from the coarse route's cost estimate × 1.3.
- **Gain:** typically 2–4× less work for nets whose price landscape changed little (estimate).
- **Risk:** none for correctness with ε = 0. An ε > 0 makes it approximate.
- **Ratings:** value **H**, effort **L**, GPU 100%.

### Idea 5: Batched multi-search dispatch (search table, arena allocator, GPU-side scheduling)
- **Refs:** general GPU graph-framework practice (Gunrock, PPoPP 2016) [M]; InstantGR, ICCAD 2024 / TCAD 2025 (batches of nets, 3D fine-grained overlap checking, more nets per batch, >13× over multithreaded) [W].
- **Mapping:**
  - `search_table[s]`: replica, net, class, window origin, tiles-in-corridor bitmap offset, arena offset, UB, status.
  - `work_list` holds (s, tile) pairs across all searches, so one dispatch covers every search of every replica.
  - A GPU-side `schedule` writes indirect arguments. The CPU encodes, say, 64 outer steps blindly and reads back a "done" count once.
  - Backtrace (1 thread per search following 4-bit parent pointers, roughly 0.3 µs per step in L2) and stamping (Idea 6) happen in the same command buffer.
- **Why it matters:** the 1650 has 14 SMs. A single net's corridor gives about 20–60 active tiles per step, far too few to fill the GPU. 16 replicas × 8–16 nets fill it.
- **Risk:** complexity of the allocator and load imbalance: one long net keeps its batch step waiting. Mitigate by letting searches finish asynchronously and refilling the table.
- **Ratings:** value **H** (necessary), effort **M**, GPU fraction ≈ 90% (the CPU only orchestrates).

### Idea 6: Reversible class-aware occupancy via dilated-footprint stamping
- **Refs:** PathFinder (McMurchie & Ebeling, FPGA 1995) [M]; this is a standard occupancy approach adapted to GPU atomics.
- **Clearance model:** with K ≤ 4 net classes, define for each viewing class c the centre-to-centre keep-out radius from a track of class k: `r(c,k) = w_c/2 + clr(c,k) + w_k/2`.
- **Stamp (on commit or rip):**
  - For net n of class k, rasterise the union of capsules along its lattice path with radius r(c,k), for each c, into a per-search scratch bitmask. The union matters: it gives "one net counts once".
  - Then `atomicAdd` (commit) or `atomicSub` (rip) of `1 << (8c)` into `occ[node]`, a u32 packing 4 × u8 counts, one per viewing class.
  - Via footprints are stamped on every layer the via spans, with the via radius.
- **Result:** `occ_c(v)` is the number of distinct foreign nets whose keep-out covers a class-c centreline at v, which is exactly the PathFinder usage with capacity 1.
- **No self-exclusion problem:** a rerouting net is ripped first. Inside a multi-pin search its own tree is never in `occ`.
- **Step cost** for class c at v: `(b + h(v)) · (1 + pf·occ_c(v)) + static_c(v)`.
- **Static field:** pads, keepouts, board edge, other nets' pours. Compute it once with JFA or brute-force gather (Idea 17) as the two nearest distinct static owners with their edge distances. It is conflicting when the nearest owner ≠ n and its distance < `clr(c, class(owner)) + w_c/2`.
- **Via placement check:** sample `occ` on a ring at (r_via − w_c/2) plus the centre (9 taps). An exact alternative is a second `occ_via` field (+48 MB per replica).
- **Memory per replica:** occ 4 B × 12 M = 48 MB, history f16 × 12 M = 24 MB, so ~72 MB (+48 MB with `occ_via`). Shared static fields: ~100 MB.
- **Quality:** same model as the CPU stamping. Lattice discretisation means the CPU exact verification must remain.
- **Risk:** u8 overflow in hot spots; saturate at 255 by clamping before the add, or use u16 pairs. Stamping cost is trivial (a ~2000-node path × 16-node-wide band × 4 classes ≈ 128 k writes).
- **Ratings:** value **H**, effort **M**, GPU 100%.

### Idea 7: Vias and layer transitions inside the wavefront
- **Mapping:** covered in Ideas 1 and 6: a per-column layer min-plus pass inside each tile. Blind or buried rules come from a mask of allowed layer spans.
- **Via cost** is a per-replica parameter, which is useful for diversity.
- **Via reduction phase:** rerun negotiation with a rising via cost, or use Idea 18.
- **Ratings:** value **H** (necessary), effort **L**.

### Idea 8: Multi-pin nets: one Voronoi wavefront (Mehlhorn) instead of k−1 successive searches
- **Refs:** Mehlhorn, IPL 1988 (2-approximation via Voronoi regions instead of all-pairs shortest paths) [W, via citing paper]; LLNL distributed 2-approximate Steiner trees (JPDC 2023) [W]; *Accelerating Computation of Steiner Trees on GPUs*, IJPP 2021 [W, title only].
- **Mapping:**
  1. Seed all terminals with `dist = 0` and their terminal label, packed as 24 bits of distance + 8 bits of label, or kept in a side buffer. Use Idea 1 with label propagation on improvement.
  2. A kernel over window edges (u, v) with `label(u) ≠ label(v)` computes `d(u) + w + d(v)` and does `atomicMin` into a per-net label-pair table (k ≤ 32 gives 1024 entries). Nets with more than ~64 pins are usually power nets; handle them with pours or on the CPU.
  3. Build the MST on the label graph (tiny: CPU, or one workgroup).
  4. Backtrace every MST edge through both Voronoi regions in parallel, one thread per edge.
  5. Optional improvement: remove the longest tree edge and reconnect with a multi-source search from the rest of the tree (1–2 extra wavefronts).
- **Alternative (closer to the current behaviour):** successive growth, multi-source from the tree to all remaining pins, connecting the nearest pin. That is k−1 searches, each with the whole tree as source. Batched Prim: after one search, connect all pins whose backtraced paths are disjoint, not just the nearest one.
- **Quality:** Mehlhorn trees are a few percent longer than successive growth (estimate, not verified). Negotiation absorbs part of the difference.
- **Pours:** a net's own pour is a terminal region.
- **Risk:** bad Voronoi boundaries in congested regions. Keep successive growth as a per-replica option.
- **Ratings:** value **M–H** (saves about 2× of search passes on typical 3–5-pin nets), effort **M**, GPU ≈ 95%.

### Idea 9: Coarse-to-fine negotiation with GPU corridors (multilevel)
- **Refs:** CUGR (DAC 2020) [W, indirect]; FastGR, DATE 2022 / TCAD 2023 (GPU pattern routing plus a heterogeneous task graph, 10.9× on GPU for pattern routing) [W]; Lin & Wong, *Superfast Full-Scale GPU-Accelerated Global Routing*, ICCAD 2022 [W]. I believe this is the "GGR" you mean, but that mapping is unverified.
- **Mapping:**
  - Level 0 is the 16×16 tile graph: 12 k tiles per layer, ~47 k nodes at 4 layers. Capacities come from the free track count across tile edges per class.
  - Run full PathFinder at level 0 on the GPU with all nets at once (Jacobi) plus the damping of Idea 12. Coarse searches cost about 47 k nodes × a few sweeps each, i.e. about 5 ms per iteration for all nets.
  - Optional level 1 at 4×4 nodes.
  - The fine level searches only the corridor tiles (route ± 1 tile). If the fine search fails or the UB is violated, widen to ± 2 tiles, then the full bounding box.
- **Quality:** corridors cut runtime greatly and cost a few percent of wirelength. Coarse capacity misestimates cause fine-level failures; history from the fine level can be fed back into coarse costs.
- **Replicas:** coarse runs are cheap enough for 64+ replicas per dispatch, which gives diverse corridors.
- **Risk:** coarse capacity modelling with mixed classes and vias.
- **Ratings:** value **H**, effort **M** (you already have the tile graph), GPU ≈ 100%.

### Idea 10: Small Gauss–Seidel batches with corridor-overlap colouring (not bounding boxes)
- **Refs:** ParaDRo (Hoo & Kumar, FPGA 2018): nets in a spatial partition run in parallel when bounding boxes do not overlap, multi-sink nets are split into single-sink connections, 5.4× on 8 threads [W]; InstantGR's 3D fine-grained overlap batching [W].
- **Mapping:**
  - Corridors are thin tile sets, so build the overlap graph on corridor tile bitsets (GPU: stamp corridor ids per tile, then pairwise via a hash; or CPU, only 200 nets).
  - Greedily colour it in a random order. Each batch is capped at B searches per replica.
  - Split multi-pin nets into connections to get more disjointness.
- **Quality:** equivalent to serial within a batch when corridors are disjoint. Long nets overlap only at their crossing tiles, so allow overlap but order commits (Idea 11).
- **Risk:** dense boards produce long colour chains, which Idea 11 addresses.
- **First experiment (CPU, cheapest and most informative):** in the existing router, route batches of B nets Jacobi-style (same snapshot, then stamp all), with B ∈ {1, 2, 4, 8, 16, 32, all}. Plot vias, time and iterations against B. This gives the batch-size quality curve the whole GPU design depends on.
- **Ratings:** value **H**, effort **L**, GPU fraction n/a (scheduling).

### Idea 11: Optimistic "route, then commit winners" (deterministic reservations)
- **Refs:** Blelloch, Fineman, Gibbons, Shun, *Internally Deterministic Parallel Algorithms Can Be Fast*, PPoPP 2012 (deterministic reservations) [M]; the speculative parallel-routing literature for FPGAs [M].
- **Mapping:**
  - Route a batch (say 32 connections per replica) against the snapshot.
  - Each new path writes `atomicMin(reserve[node], priority)` over its keep-out footprint (random priority per replica, or improvement-weighted).
  - A second pass keeps a path if it holds all its reservations, or only loses them to nets it did not conflict with in the snapshot.
  - Winners are stamped. Losers are rerouted in the next sub-round against the updated `occ`. They are already ripped, so they keep their old route only if they still lose after 2 tries.
- **Quality:** a deterministic analogue of Gauss–Seidel. The number of sub-rounds adapts to the actual interaction density.
- **Risk:** starvation of long nets. Use age-based priority.
- **Ratings:** value **H**, effort **M**, GPU 100%.

### Idea 12: Jacobi done properly: random subsets, damped prices, price noise, Lagrangian view
- **Refs:** ParaLarH, a parallel FPGA router based on Lagrange heuristics (arXiv 2010.11893) [W, title only]; PathFinder [M]. Oscillation in simultaneous updates is a well-known fictitious-play or subgradient phenomenon [M].
- **Knobs:**
  1. Each conflicted net reroutes this round with probability ρ ∈ [0.1, 0.3]. That is damped Jacobi.
  2. Present cost uses smoothed occupancy `ô ← α·occ + (1−α)·ô`.
  3. Slower present-factor growth.
  4. Per-net multiplicative log-normal noise on base costs (σ ≈ 0.05–0.1), which breaks the symmetry of "everyone flees to the same gap".
  5. History updates stay as in PathFinder.
- **Quality:** much better than naive Jacobi, but probably still worse than Gauss–Seidel in the endgame. Use it early and at the coarse level, then switch to Ideas 10 and 11.
- **Replicas:** noise seeds and ρ are natural replica parameters.
- **Cheap experiment:** repeat your Jacobi experiment on the CPU with ρ = 0.25 and noise, and compare with 448 s / 44 vias.
- **Ratings:** value **M–H**, effort **L**.

### Idea 13: Replica portfolio in one dispatch, with successive halving
- **Refs:** successive halving / Hyperband (Jamieson & Talwalkar 2016; Li et al. 2017) [M].
- **Mapping:**
  - R replicas, each with its own `occ`, history and routes (~72–120 MB each), share the static fields and the scratch arena. On 4 GiB: R ≈ 16 at 3 M nodes per layer, R ≈ 8 at 6 M.
  - Parameters that differ per replica: net order and priority seed, noise σ, via cost schedule, present-factor growth, batch size, corridor width, multi-pin method, coarse corridor seed.
  - After ~5 iterations, drop the worst half (by overflow plus via count) and reuse their memory to fork the best replicas with new seeds. Repeat.
- **Quality:** best-of-N with pruning. This is the axis that does not damage convergence.
- **Risk:** the variance between configurations may be small on easy boards. Measure first.
- **First experiment:** run the existing CPU router on N cores with N seeds and parameter mixes. Measure the spread of via count and time, and how early the ranking is predictive.
- **Ratings:** value **H**, effort **L–M** (given Idea 5), GPU 100%.

### Idea 14: Generate candidate routes, then select them concurrently (DGR-like)
- **Refs:** DGR (Li et al., DAC 2024): GPU differentiable concurrent global routing over a routing DAG forest for hundreds of thousands of nets [W].
- **Mapping:**
  - Generate about 8–32 diverse candidate routes per connection: multiple searches with perturbed prices, or k-diverse paths from a penalised second search. This is embarrassingly parallel.
  - Build candidate-pair conflicts by stamping candidate ids into a spatial hash.
  - Select one candidate per connection to minimise conflicts + wirelength + vias, either by softmax relaxation plus gradient descent, or by parallel tempering over the replicas (each replica is a chain).
  - Iterate: new candidates for connections that still conflict.
- **Quality:** strong for resolving global contention. Limited by the candidate set, so hard bottlenecks still need negotiation.
- **Memory:** 400 connections × 32 candidates × ~500 lattice nodes as run-length compressed paths ≈ tens of MB.
- **Risk:** the size of the conflict graph and the design of the objective.
- **Ratings:** value **M–H**, effort **H**, GPU ≈ 90%.

### Idea 15: GPU congestion pre-estimation from probabilistic pattern routes
- **Refs:** FastGR's GPU pattern routing and CUGR's probabilistic resource model [W].
- **Mapping:** for each connection, spread fractional demand along L, Z and 45° pattern shapes, with atomicAdd into a coarse demand map (one kernel, well under 1 ms). Seed history costs and coarse capacities from overflow before the first negotiation iteration.
- **Gain:** fewer negotiation iterations and better initial ordering (estimate).
- **Ratings:** value **M**, effort **L**, GPU 100%.

### Idea 16: Multi-net coloured wavefront ("Voronoi competition")
- **Mapping:** all nets' terminals are seeded at once with net-id labels. Each node is claimed by the first-arriving net (`atomicMin` on packed dist|net). Then backtrace.
- **Honest assessment:** it is greedy and has no negotiation. Nets whose paths must cross another net's territory lose. It is useful for:
  - territory-based initial corridors,
  - spotting bottleneck regions (Voronoi boundaries that many connections must cross),
  - ordering heuristics.
  It is not a router.
- **Ratings:** value **L–M**, effort **L**, GPU 100%.

### Idea 17: Distance transforms for static fields and soft costs
- **Refs:** Rong & Tan, *Jump Flooding in GPU*, I3D 2006 (approximate Voronoi and distance transform, log n passes) [W]; Felzenszwalb & Huttenlocher, *Distance Transforms of Sampled Functions*, ToC 2012 (exact, separable, linear) [M].
- **Uses:**
  - A 2-nearest-distinct-owner transform for static copper (JFA with top-2 per node, or an exact separable 1D lower-envelope pass per owner class). Computed once; about 12 passes × 12 M nodes.
  - A soft DFM cost that keeps tracks off copper edges (a function of the distance).
  - Pad-escape cost fields.
  - Quick lower bounds h for Idea 4: a distance transform from the target through static obstacles gives an admissible, obstacle-aware heuristic that is much tighter than octile distance.
- **Ratings:** value **M** (**H** for the heuristic field), effort **L**, GPU 100%.

### Idea 18: Via reduction and layer assignment as a GPU dynamic programme
- **Refs:** GAP-LA, GPU-accelerated performance-driven layer assignment (arXiv 2507.13375, 2025) [W, title only]; DP layer assignment in CUGR [W, indirect].
- **Mapping:**
  - Keep each net's 2D projection fixed and choose a layer per node or segment.
  - The cost is the sum of per-layer occupancy costs plus via cost times the number of layer changes.
  - This is a Viterbi pass along the path with L = 4 states. It is associative as a tropical 4×4 matrix product, so it can be a log-depth parallel scan, or simply one workgroup per net.
  - Trees: DP over the tree (children first).
  - Run it on a random subset of nets per round (Jacobi is safe here because the changes are local), then re-stamp.
- **Quality:** fast via removal. It cannot move the xy path, so final rerouting with a rising via cost may still be needed for a few nets.
- **Risk:** SMD pads force top or bottom endpoints, which is fine as a boundary condition. Via footprint conflicts need the `occ_via` check.
- **Ratings:** value **M–H**, effort **L–M**, GPU ≈ 100%.

### Idea 19: Parallel 45° / any-angle post-processing
- **Mapping, per net:**
  1. Mark direction-change points in parallel and compact them to get the lattice polyline.
  2. Build a visibility DAG over the polyline vertices within a window k (for example 64): edge i→j exists if the segment has an allowed angle (0/45/90, or any angle) and passes the clearance field. Test segments one thread per (i, j), sampling `occ_c` and static every ½ pitch.
  3. Run a DP for the fewest segments, then the shortest length (one thread per net, or a tropical scan).
  4. Pull corners: replace a 90° corner with two 45° chamfers when clear.
- **Quality:** about the same as serial smoothing, and exact DRC follows anyway.
- **Ratings:** value **M**, effort **L–M**, GPU ≈ 80% (DP on the CPU is also fine; this is small work).

### Idea 20: GPU prefilter for exact verification
- **Mapping:** capsule–capsule and capsule–pad tests over a uniform spatial hash (bin = max keep-out), one thread per bin pair. The CPU exact check runs only on flagged pairs.
- **Ratings:** value **L–M** (only if verification is a notable fraction of time), effort **L**.

### Idea 21: Hotspot windows for endgame renegotiation (spatial decomposition)
- **Refs:** *Coarse-Grained Parallel Routing With Recursive Partitioning for FPGAs* (TPDS) [W, title only]; ParaDRo [W].
- **Mapping:**
  - Late iterations leave a few conflict clusters. Extract windows of about 64×64 tiles around each cluster, merging windows that overlap.
  - In each window, all nets touching it are rerouted inside it with serial PathFinder: the nets of one window run in sequence, each search a full GPU wavefront.
  - Disjoint windows and replicas run in parallel.
  - The window boundary pins the entry and exit points; relax that by one tile.
- **Quality:** serial-quality local negotiation, with parallelism across windows × replicas.
- **Ratings:** value **M–H**, effort **M**.

### Idea 22: Direction-augmented state for bend penalties
- **Mapping:** state (node, incoming direction), i.e. 8× dist inside the window only. Relaxation adds a turn penalty; 45° turns are cheap and 90° turns expensive. Four directions are enough if 45° turns are free.
- **Memory:** a corridor window of 0.3 MB becomes 2.4 MB per search. At 128 concurrent searches that is ~300 MB, which fits.
- **Quality:** fewer jogs, which makes Idea 19 easier and lengthens routes less.
- **Ratings:** value **M**, effort **M**, GPU 100%.

### Idea 23: Hybrid tail on the CPU
- **Refs:** FastGR's heterogeneous CPU/GPU task graph [W].
- **Mapping:** when fewer than about 8 conflicted connections remain in a replica, GPU overhead dominates. Hand that replica's last conflicts to the existing CPU A* (serial, exact semantics) while the GPU continues other replicas.
- **Ratings:** value **M**, effort **L**.

**Prior PCB GPU router: OrthoRoute** (Benchoff) [W]. It uses a CUDA SSSP ("parallel Dijkstra") inside each net's search, but routes nets sequentially on a shared congestion map, on a Manhattan per-layer-direction lattice for backplanes. It is a useful sanity check that intra-search parallelism alone gives limited speedup. The batching and replica axes above are what it lacks.

---

## 2. Where sequential dependence is essential, and how to break it

- **Why it matters.** PathFinder converges because each net sees current prices. The damage comes from contention on the same scarce resource by several simultaneously updated nets: they herd to the same alternative and oscillate.
- **What does *not* need sequencing:**
  - searches whose corridors are disjoint,
  - different replicas,
  - multi-pin Voronoi terminals within a net,
  - layer-assignment DP (local),
  - post-processing.
- **What does:** crossing or competing nets in hot regions.
- **Breakers, from most to least faithful to serial:**
  1. Replicas: no loss.
  2. Corridor-coloured small batches with ordered commits (Ideas 10 and 11): near-serial.
  3. Hotspot windows with serial local negotiation (Idea 21).
  4. Damped, randomised Jacobi with price noise (Idea 12): acceptable early and at the coarse level.
  5. Full Jacobi: already shown to be bad.
- **Choosing B:** B should be set from the CPU batch-size curve (the Idea 10 experiment). My prior is that B ≈ 8–16 costs less than about 10% in vias. This is unverified.

---

## 3. Top 5

1. **Batched tile-asynchronous corridor wavefront kernel** (Ideas 1+2+3+4+5): the engine. It makes one search cheap and many searches share a dispatch.
2. **Replica portfolio × small ordered Gauss–Seidel batches** (Ideas 13+10+11): this puts parallelism where it does not hurt convergence, and it matches "many randomised runs, pick the best".
3. **Reversible class-aware occupancy stamping plus static 2-nearest fields** (Ideas 6+17): it makes widths, clearance, vias and pads exact enough on the lattice with u32 atomics, keeps memory to about 72–120 MB per replica, and supplies an obstacle-aware heuristic.
4. **GPU coarse negotiation → corridors** (Ideas 9+15): it shrinks fine work by an order of magnitude and makes corridors thin enough that batches are mostly disjoint.
5. **GPU via reduction by layer DP plus hotspot-window endgame** (Ideas 18+21), with a CPU tail (Idea 23): this replaces the expensive serial tail phases.

---

## 4. End-to-end pipeline (4 layers, 200 nets, 3 M nodes per layer, GTX 1650)

**Assumptions:**
- Board lattice about 2000×1500 nodes at ~0.07 mm; tiles 16×16×4, so ~11.7 k tile-columns.
- About 400 two-pin-equivalent connections, average 25 mm ≈ 360 nodes ≈ 23 tiles.
- Corridor ±1 tile, ~50 tile-columns after pruning, ~2.5 visits each, so ~150 tile-visits per connection.

**Memory:**

| Item | Size |
|---|---|
| Shared static fields (2-nearest owners, obstacle-aware h per target is computed per search, not stored) | ~100 MB |
| Per replica: `occ` u32 48 MB + history f16 24 MB + routes ~2 MB | ~74 MB |
| Per replica with `occ_via` | ~120 MB |
| Arena: 256 concurrent searches × ~0.3–0.6 MB | ≤150 MB |
| **Total at R = 16 (with `occ_via`)** | **≈ 2.2 GB, fits 4 GiB** |

**Stages:**

- **S0, setup (once, GPU):**
  - Rasterise pads, keepouts, edges and fixed pours.
  - JFA 2-nearest transform: about 12 passes × 12 M × ~72 B ≈ 10 GB of traffic, **~80 ms**.
  - Per-class static conflict masks are computed on the fly.
- **S1, coarse (GPU, R_c = 64 replicas):**
  - Pattern-route demand (Idea 15), then coarse PathFinder over the ~47 k-node tile graph with damped Jacobi.
  - About 5 ms per iteration per replica-group, ~30 iterations: **~150–300 ms total**.
  - Keep the best R = 16 corridor sets.
- **S2, fine negotiation (GPU, R = 16 in lockstep):**
  - Per iteration, per replica:
    - (a) Choose the conflicted set (all connections at iteration 1, then ρ-subset or all conflicted).
    - (b) Colour by corridor overlap, B ≤ 16.
    - (c) For each batch step: rip (atomicSub stamps); tile-asynchronous search with A*-ordered tile-Δ scheduling and UB pruning, ~60 outer steps of 2 dispatches each; backtrace; reservation check (Idea 11); commit stamps; losers go into the next step.
    - (d) History update: one 12 M-node pass, ~1 ms.
  - Work per connection:
    - compute: 150 visits × 1024 nodes × ~60 relaxations ≈ 9 M relaxations at ~5 ops each, about 45 µs of whole-GPU time at an effective ~1 T simple ops/s;
    - memory: 150 × ~20 KB = 3 MB ≈ 23 µs;
    - so **≈ 50 µs per connection** (compute-bound).
  - Full-reroute iteration (400 connections):
    - work ≈ 20 ms per replica;
    - batch steps: 400/16 = 25, each ≈ 60 steps × 2 dispatches × ~10 µs bubble ≈ 1.2 ms, so **~30 ms overhead per iteration, shared by all replicas**;
    - backtrace and stamps ≈ 25 × 0.2 ms = 5 ms, shared.

    | | R = 1 | R = 16 |
    |---|---|---|
    | Full-reroute iteration | ≈ 55 ms | ≈ 16 × 20 + 35 ≈ **355 ms (~22 ms per replica)** |
    | Later iterations (~20% conflicted) | ≈ 4 + 12 = 16 ms | ≈ 64 + 12 ≈ **~80 ms** |

  - About 50 iterations (3 full + 47 partial): R = 1 **≈ 1.0 s**, R = 16 **≈ 5 s**.
  - Successive halving at iterations 5 and 15 cuts this further, or lets you fork new seeds instead.
- **S3, via reduction (GPU):**
  - Layer DP (Idea 18) on random 30% subsets, about 1 ms per round per replica-group, ~10 rounds.
  - Then about 5 renegotiation iterations with rising via cost: **~0.5 s at R = 16**.
- **S4, endgame:**
  - Hotspot windows (Idea 21) on the GPU.
  - Fewer than 8 conflicts → CPU A* tail per replica, in parallel threads: **~0.5–2 s**.
- **S5, finish (CPU for the top 2–3 replicas):**
  - 45° / any-angle smoothing (Idea 19, GPU segment tests optional), cleanup, exact verification, KiCad DRC: **seconds**, as today.

**Rough total:** about 16 configurations in **~10 s of GPU time plus a few seconds of CPU**, against 5–20 min for one serial run today. Uncertainty is about ±3×. The main unknowns are:
- tile revisit counts in mazes,
- the real dispatch-bubble cost in wgpu/Vulkan on the 1650,
- how many iterations small-batch Gauss–Seidel needs compared with serial.

At 0.05 mm pitch (6 M nodes per layer), work per connection roughly doubles and R drops to ~8, so expect about the same wall time with half the replicas.

**Order of experiments:**
1. CPU batch-size curve (Idea 10).
2. CPU seed-portfolio spread (Idea 13).
3. Single-search wgpu tile kernel against CPU A*.
4. Batched 128-search dispatch throughput.
5. Stamping plus a full single-replica GPU loop.

---

## References

- GAMER, TCAD 2023 [W] https://ieeexplore.ieee.org/document/9799536/
- InstantGR, ICCAD 2024 / TCAD 2025 [W] https://dl.acm.org/doi/10.1145/3676536.3676787 ; code https://github.com/cuhk-eda/InstantGR
- Superfast Full-Scale GPU-Accelerated Global Routing, ICCAD 2022 [W] https://dl.acm.org/doi/10.1145/3508352.3549474
- FastGR, DATE 2022 / TCAD 2023 [W] https://yibolin.com/publications/papers/ROUTE_DATE2022_Liu.pdf
- DGR, DAC 2024 [W] https://dl.acm.org/doi/10.1145/3649329.3656530
- GAP-LA [W, title] https://arxiv.org/pdf/2507.13375
- Corolla, FPGA 2017 [W] https://dl.acm.org/doi/10.1145/3020078.3021732
- Shen, Luo, Xiao, TPDS 2019 [W] https://ieeexplore.ieee.org/document/8567949/
- ParaDRo, FPGA 2018 [W] https://dl.acm.org/doi/10.1145/3174243.3174246
- MultiQueue FPGA routing, FPT 2024 [W, title] https://www.eecg.utoronto.ca/~mcj/papers/2024.mqrouter.fpt.pdf
- ParaLarH [W, title] https://arxiv.org/pdf/2010.11893
- Davidson et al., IPDPS 2014 [W] https://escholarship.org/uc/item/8qr166v2
- ADDS, PPoPP 2021 [W] https://www.cs.utexas.edu/~lin/papers/ppopp21.pdf
- Teodoro et al., irregular wavefront propagation [W] https://arxiv.org/pdf/1209.3314
- Jump flooding, I3D 2006 [W] https://www.comp.nus.edu.sg/~tants/jfa.html
- Distributed 2-approximate Steiner / Mehlhorn [W] https://www.osti.gov/servlets/purl/2007614
- GPU Steiner trees, IJPP 2021 [W, title] https://link.springer.com/article/10.1007/s10766-021-00723-0
- OrthoRoute [W] https://github.com/bbenchoff/OrthoRoute
- Not web-checked this session [M]:
  - Meyer & Sanders, delta-stepping, 2003
  - Gunrock, PPoPP 2016
  - Felzenszwalb & Huttenlocher, 2012
  - Zhao, fast sweeping, 2005
  - McMurchie & Ebeling, PathFinder, 1995
  - Blelloch et al., deterministic reservations, PPoPP 2012
  - Hyperband / successive halving
  - CUGR (DAC 2020; only seen indirectly)
