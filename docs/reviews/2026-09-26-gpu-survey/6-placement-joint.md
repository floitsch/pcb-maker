# GPU placement and joint place+route for a small-board KiCad flow: ideas, top 5, and one pipeline

## 0. Five things that shape everything below

1. **One PCB placement cannot keep a GPU busy.** A board has about 250 bodies and 3k pins, so each kernel is dominated by launch and dispatch cost. The GPU earns its keep in two ways only:
   - **Batched replicas**: B = 128–1024 placements solved together as one set of [B,N] tensors. 256 × 250 bodies is 64k bodies, which is ISPD-benchmark scale and where DREAMPlace/Xplace-style kernels start to pay off.
   - **Raster fields**: routing demand, capacity, distance and congestion maps at 0.1–0.5 mm.

   Design every buffer with a leading replica dimension from the start.
2. **wgpu/WGSL is not CUDA.** Plan for these gaps:
   - No portable float atomics. Use i32 fixed-point `atomicAdd`, or gather-style kernels. A float-atomic native feature exists on some backends; check it (unverified).
   - No cuFFT. You write a batched DCT/DST yourself (radix-4/16 stockham).
   - The default `maxStorageBufferBindingSize` is 128 MiB. Request the adapter's real limits.
   - No autodiff. Either hand-write adjoint kernels, or look at Burn/CubeCL, which has a wgpu backend and autodiff (unverified for this use).
   - Prototype in PyTorch/CUDA first, then port.
3. **"Routability" on a 2–4 layer PCB is not VLSI routability.** There are no preferred-direction tracks, pads are large, pin access dominates, and on 2-layer boards the count of ratsnest crossings and vias matters more than the RUDY map. Any estimator must be checked against *your* router before you trust its gradient.
4. **Pinless obstacles get a force only if routing capacity depends on body pose.** Make the blocked capacity of each routing cell a soft-rasterized, differentiable function of every body's pose. Congestion then pushes all bodies, connected or not. This is the single most important modelling change.
5. **Exact legality and DRC is never done on the GPU.** The GPU gets you to "nearly legal, conservatively rasterized". A CPU geometric pass (jagua-rs for courtyards, a polygon clearance check, local repair, KiCad DRC) closes the gap. Make the raster *conservative* so this pass rarely has work to do.

## 1. Literature anchors

- **Cypress** (Zhang, Agnesina, …, Ren; ISPD'25 Best Paper; NVlabs/Cypress, Apache-2.0). Verified. A GPU PCB placer built on DREAMPlace. It has separate density maps for top and bottom, component rotation, and a smoothed **net-crossing** cost as a routability proxy. It is the closest prior art: read its cost functions before writing your own.
- **DREAMPlace**. Verified. Routability via RUDY/pin-utilization cell inflation (`adjust_node_area`). DREAMPlace 4.0 is *timing* (momentum net weighting), not routability. Liu et al., DATE'21, add a CNN congestion predictor inside DREAMPlace (verified).
- **ABCDPlace** (TCAD'20). Verified. Batch-concurrent GPU detailed placement: independent-set matching, global swap, local reorder.
- **Xplace 2.0 / 3.0** (CUHK). Verified. GPU detailed-routability-driven global placement, integrated with the **GGR** GPU global router (Lin & Wong, ICCAD'22, verified).
- **InstantGR** (ICCAD'24, TCAD'26). Verified. Batched GPU 3D global routing with fine-grained overlap checking.
- **GAMER** (ICCAD'21 / TCAD'23). Verified. GPU maze routing as alternating horizontal and vertical sweeps, O(log n) per sweep via parallel scan. This is the kernel model for a wgpu wavefront router.
- **DGR** (Li et al., DAC'24; NVlabs). Verified. A differentiable concurrent global router: soft selection over a DAG forest of Steiner and 2-pin pattern candidates. Pins are fixed; it has no gradient with respect to placement. Extending it to pin positions is new work.
- **RUPlace** (Chen et al., DAC'25). Verified. Unifies placement and routing with ADMM, Wasserstein distance and bilevel optimization, alternating global routing and incremental placement. It also picks cell-inflation ratios by convex optimization. The closest "joint" formulation in VLSI.
- **Differentiable Net-Moving and Local Congestion Mitigation** (DAC'25). Verified. Momentum-based cell inflation, plus a Poisson-derived global congestion function with virtual cells on 2-pin nets.
- **Apollo** (ICCAD'25). Verified by abstract only. GPU routing-informed placement for photonic ICs with explicit crossing and bend modelling. Its crossing modelling is relevant to 2-layer PCBs.
- **Differentiable Routability-Driven Package Floorplanning with Pin Assignment** (arXiv 2607.15005). Verified by abstract only. Differentiable discrete orientations and a GPU-parallel crossing-aware cost; fan-out congestion estimated with Top-K paths.
- **ULTRA-2.5D**. Verified by snippet. Gumbel-softmax relaxation of orientation and flip in a differentiable chiplet placer.
- **ChipDiffusion** (Lee et al., ICML'25). Verified. A diffusion model trained on synthetic data places macros zero-shot, with guided sampling for legality and wirelength.
- **jagua-rs** (Gardeyn et al., INFORMS JoC) and **sparrow** (arXiv 2509.13329). Verified. Both Rust. jagua-rs is an exact collision engine for irregular polygons with continuous rotation. Sparrow's overlap proxy uses "poles" (inscribed circles) and is smooth, cheap and monotone in overlap.
- **XPBD** (Macklin, Müller, Chentanez, MIG'16). Verified.
- **Li & Milenkovic LP compaction** (1995). Unverified in this session.
- **Rubber-band / topological routing** (Dai et al., SURF, early 1990s). Unverified.

## 2. Ideas (23)

**Placement core**

1. **Batched-replica ePlace.** All state is [B,N,·].
   - Each replica has its own seed, λ-schedule, target density, halo scale, WA γ, H/V layer preference and initial side assignment.
   - Kernels: pose→pin positions; WA wirelength with one workgroup per (replica, net); density splat; batched DCT; field sampling; Nesterov step.
   - With B=256 and 1 mm bins, one iteration takes about 5–15 ms on a 1650, dominated by the DCTs.
   - This is 100% GPU and is the base for everything else.
2. **Two-scale density.**
   - Keep a coarse FFT electrostatic field (1–2 mm bins, one map per side; through-hole parts charge both maps) for *global* spreading. A 64-bin grid is too coarse for 150 mm boards; use 128–256.
   - Add a *pairwise* exact-ish overlap energy (pole or SDF proxy) that ramps in once overflow drops below about 0.3. With 250 bodies, O(N²) pairs is only 31k pairs per replica.
   - FFT-only ePlace leaves large bodies overlapping at the end; pairwise-only gets stuck in local minima. The combination fixes both.
3. **Poles as the body representation.**
   - Approximate each courtyard by K ≤ 16–32 inscribed discs (sparrow's pole construction). Keep the exact polygon for the final check.
   - Rotating a body only rotates the disc centres, so arbitrary angles cost nothing.
   - The density splat becomes a disc/Gaussian splat.
   - Torque comes from the field sampled at pole centres: τ = Σ rᵢ × Fᵢ.
   - The pairwise overlap proxy becomes circle–circle penetration, which is branch-free and suits WGSL.
4. **Continuous rotation with a quantization penalty.**
   - Make θ a free variable. The wirelength gradient is ∂p/∂θ = R′(θ)·offset.
   - Add λ_q·sin²(2θ) and anneal λ_q upward so bodies settle at 0/90/180/270°. Leave it off if arbitrary angles are allowed.
   - 0° versus 180° is separated by a penalty barrier, so handle 180° flips as discrete replica moves (idea 5).
   - Gumbel-softmax over four orientations gives "expected pin positions" that shrink toward the body centre and bias wirelength. Use it only straight-through, if at all.
5. **Side choice as a discrete parallel-tempering move plus an area pre-solve.**
   - Continuous side relaxation mixes mirrored pad sets and pad layers, which is physically meaningless for routing.
   - Instead:
     - Solve a tiny side-assignment problem first (greedy or ILP, per side: SMD area ≤ α·usable area, with through-hole footprints counted on both sides). This fixes the "dense two-sided board has no legal placement" case up front: if the problem is infeasible, you know before running anything.
     - Then run Metropolis flip moves across replicas at different temperatures, and swap replicas periodically.
6. **Pin-access halos in place of pin-count halos.**
   - Inflate by *escape demand*: pads × required track and via pitch versus the perimeter capacity of the body, per layer.
   - Update with momentum from measured congestion (DAC'25 DCGP-style, verified). This avoids the "inflate, then move back into the hot spot" oscillation.
7. **Functional-cluster initializers for diversity.** Spectral embedding of the net hypergraph, with decoupling capacitors pre-attached to their IC power pins and noise added. Seeds then differ in cluster arrangement and side assignment, not just in jitter.

**Routability inside the placement loop**

8. **Differentiable capacity; this is the fix for pinless obstacles.**
   - cap_l(g) = base_l(g) − Σ_b softblock_b,l(g; pose_b), where softblock is sigmoid(−SDF_b(g)/τ) plus clearance.
   - Then ∂overflow/∂pose_b = Σ_g ∂overflow/∂cap(g) · ∂cap/∂pose_b.
   - A part with no nets sitting in a hot channel now gets pushed out.
   - Gradients come from the SDF gradient (a 64×64 SDF texture per footprint, about 8 KB f16).
9. **DGR-lite with gradients with respect to pins.**
   - For each replica, grid at 0.5 mm GCells × L layers. Decompose each net into 2-pin connections (MST per net, or 1-Steiner every ~50 iterations).
   - Candidates per connection: two L shapes plus about three Z shapes per direction, times a layer or via assignment, about 8–16 in total. Softmax logits with temperature give the selection.
   - demand = Σ p_c · raster(c). Loss = Σ softplus(demand − cap).
   - Gradients flow to (a) the logits, as in DGR's routing step, and (b) pin positions, through *soft segment rasterization* (area-weighted or Gaussian line splat; bend positions move with the pins).
   - This alternates naturally with Nesterov steps and is the continuous "joint" core.
10. **Ratsnest crossing and via penalty for 2-layer boards.**
    - A smoothed segment-intersection indicator over pairs of 2-pin connections from different nets, following Cypress's crossing cost.
    - Cull pairs with a spatial hash over segment bounding boxes, and evaluate every ~10 iterations.
    - On 2-layer boards, crossings are a lower bound on vias and layer changes, and a better proxy than RUDY.
11. **Via-site and plane-access availability (4-layer).** Nets tied to an inner plane need a via site within about 1 mm of the pad. Build a via-site density map (free positions after pads and courtyard keep-outs). Penalize a pad whose local via-site supply is below demand, and make the penalty differentiable with respect to the neighbours' poses.
12. **Learned correction, trained on your own router.** A small CNN maps (RUDY, pin density, DGR-lite demand, capacity) to the router's actual overflow or failed connections. Train it on data generated by running the existing router over thousands of randomized placements; this is cheap and needs no external dataset. Use it as a residual on DGR-lite, not a replacement (the DATE'21 analogue).

**Joint coupling with real routes**

13. **The congestion-history field as a universal potential.**
    - After any real routing pass, take the negotiated history raster h(g) (blurred, per layer).
    - Force on body b: F_b = −∇_pose Σ_g h(g) · occ_b(g), using the same soft rasterization as idea 8.
    - It applies to every body with or without nets, is fully GPU, and replaces the heuristic "nudge congested footprints" step.
14. **Route tension.**
    - For each routed net, ∂L_route/∂pin ≈ −t̂, the unit tangent of the route where it leaves the pad.
    - Detour excess (routed length / Steiner length) scales the net's weight in the WA wirelength (momentum net weighting in the DREAMPlace-4.0 style, applied to routability).
    - This pulls pins along their real routes, not straight-line approximations.
15. **Elastic-band joint state for final refinement.**
    - Variables: body poses plus route polyline vertices, with topology fixed by the discrete router.
    - Energy: length + clearance(route–route, route–foreign copper, via SDFs) + body overlap.
    - Per-vertex Jacobi on the GPU; bodies receive equal and opposite reactions from the tracks near them.
    - Good for squeezing a few tenths of a millimetre to clear DRC after routing. It cannot change topology; crossings still need the discrete router. It is risky, so treat it as a later project.

**GPU routing**

16. **A Jacobi (all-nets-at-once) negotiated-congestion router in WGSL.**
    - Per iteration, rip up a random 20–40% subset of nets. Randomized subsets and ramped present-cost stop oscillation, and also supply diversity across replicas.
    - Wavefront each ripped-up net inside its window using GAMER-style alternating directional sweeps with prefix-min scans. Layer transitions are via-cost relaxations between sweeps.
    - Multi-pin nets grow as a multi-source tree, adding one pin at a time. Each net is sequential over its pins, but all nets run in parallel.
    - Commit the new paths, update present and history costs.
17. **Conservative per-net-class clearance maps.**
    - Rasterize foreign copper per layer.
    - Run an exact GPU distance transform (separable Felzenszwalb, one pass per row then per column).
    - Threshold at clearance + width/2 + cell·√2/2 for each net class; only a few classes exist, so only a few maps.
    - Any raster path is then DRC-clean by construction, up to pad-entry geometry.
    - Handle off-grid pad escapes geometrically on the CPU once per footprint pose.
18. **Corridor-restricted fine routing.** Route at 0.25 mm first, then restrict 0.1 mm per-net fields to the coarse path ± 0.5–1 mm instead of bounding box + 2 mm. Expect a 3–10× memory cut. This turns the measured 100 MB–1.6 GB for 4-layer boards into roughly 30–300 MB.
19. **Pour connectivity on the GPU.** Connected components of each pour net's free region by label propagation or pointer jumping, about 10 ms at 6 M cells. Islands feed back as (a) stitching-via insertion targets and (b) a routing-cost term that raises the cost of bottom-layer tracks which split the pour. This is also cheap enough to use as a placement-stage metric.

**Legalization and discrete refinement**

20. **XPBD contact legalization with SDFs.**
    - Contacts: boundary samples (32–64 points) of body A are tested against B's SDF texture, giving penetration depth and normal.
    - Only same-side pairs collide; through-hole parts collide with both sides. Fixed parts have infinite mass.
    - The outline and cutouts are a board SDF (0.1 mm, about 3 MB f16) acting as a unilateral barrier.
    - Jacobi constraint averaging in parallel over (replica, pair).
    - About 1 ms per step for 64 replicas × about 2k candidate pairs.
    - Rotation stays continuous, then snaps.
21. **Sparrow-style guided local search in each replica for dense boards.**
    - Each replica is one guided-local-search chain minimizing weighted pole overlap plus a displacement or wirelength term inside the fixed outline.
    - The GPU evaluates about 256 candidate poses (translations, quarter turns, flips) per selected body per step, in one dispatch across replicas.
    - The exact check is jagua-rs on the CPU (Rust, drop-in).
    - Leftover overlaps go to an **LP compaction**: separating-direction linearization n·(x_j − x_i) ≥ d(n) from the Minkowski support function, min Σ|Δx|, iterated. Rectangles become a min-cost-flow dual. The linearization follows Li–Milenkovic (unverified).
22. **Batch-concurrent discrete refinement.** Swaps, quarter turns and flips on independent sets (bodies sharing no nets and not in contact), scored by incremental wirelength + DGR-lite overflow on the GPU (ABCDPlace pattern).
23. **A diffusion prior, judged critically.** ChipDiffusion-style guided sampling could produce diverse initial placements, and synthetic training data is possible. But PCB placement quality is driven by pin access and functional grouping, which rules capture (decoupling capacitor near its pin, connector at the edge). There is little labelled data; scraped KiCad projects vary in quality. The guidance terms you would need (legality, wirelength, capacity) are the same as ideas 8–10 anyway. **Skip for now.** Revisit only as an initializer once the idea 12 data pipeline exists. Population search plus good estimators is the better use of GPU time.

**Glue**

- **Successive halving with estimators of increasing fidelity**: wirelength/density, then DGR-lite, then 0.25 mm Jacobi route, then 0.1 mm route.
- **Niching**: keep replicas at least D apart in body-displacement distance, which is permutation-free because bodies are labelled.
- Optionally, CMA-ES over hyperparameters across runs, and a per-board "best config" cache.

## 3. Top 5

1. **Batched replicas + poles + two-scale density + continuous θ + side pre-solve (ideas 1–5).** This is what makes the GPU worthwhile at all. It gives hundreds of diverse placements per second of GPU time, handles arbitrary rotation, and fixes infeasible dense two-sided boards up front.
2. **Differentiable capacity + DGR-lite with pin gradients + crossing term (8, 9, 10).** It attacks "the wirelength-optimal placement doesn't route" and "pinless obstacles" directly, inside the gradient loop, without calling the router.
3. **Jacobi negotiated-congestion router in WGSL with GAMER sweeps, conservative distance-transform maps and corridors (16–18).** It makes the judge 10–50× cheaper and batchable over the top K candidates; the same kernels produce the history field.
4. **History-field potential + route tension as the coupling (13, 14).** It closes the loop with *real* routes, is fully GPU, and applies to every body.
5. **XPBD/SDF, then parallel sparrow-style search, then jagua-rs exact check, then LP compaction (20, 21).** Legal placements with real courtyard polygons and continuous rotation, with a guaranteed exact final check.

## 4. End-to-end GPU pipeline

**Static buffers, built once per board (CPU → GPU):**
- Footprint table: poles (≤32 × vec3), 64² f16 courtyard SDF, pad list (offset, size, layer mask, net, net class).
- Board outline and keep-out SDF at 0.1 mm, per layer.
- Net CSR (net → pins); net-class rule table.

**Dynamic buffers:**
- Pose [B,N,4] (x, y, θ, side), plus Nesterov buffers ×3.
- Pins [B,P,2].
- Density [B,2,H,W] with φ, Ex, Ey.
- Capacity, demand and gradient [B′,L,Hg,Wg] at 0.5 mm.
- Logits [B′,C2,K].
- Router, per candidate: per-layer cost, history and owner maps at 0.25 mm and then 0.1 mm, plus per-net corridor distance windows (u16/f16).

**Stages:**

- **S0 (CPU).** Parse the board, build poles and SDFs, run the side-assignment area pre-solve and the initial spectral clusters.
- **S1 (GPU).** Batched global placement:
  - B replicas, bins at 1.5 mm and then 1 mm.
  - Per iteration: K1 pins → K2 WA → K3 pole splat (fixed-point atomics) → K4/K5 DCT, spectral solve, inverse DCT/DST → K6 field sampling (forces and torques) → K10 Nesterov step with outline barrier and fixed-part projection.
  - Every 20 iterations, read back per-replica HPWL and overflow and halve the population.
- **S2 (GPU).** Routability refinement on B′ survivors. Add K7 (soft blockage → capacity), K8 (candidate demand splat), K9 (overflow adjoint → logits, pins, bodies) and the crossing term. Alternate 1 logit step with 1 pose step, and re-run the MST/Steiner decomposition every 50 iterations. Apply the sin²(2θ) ramp and discrete flip and rotation tempering moves.
- **S3 (GPU + CPU).** Legalization on B″: XPBD SDF contacts, then guided local search with batched candidate evaluation. The jagua-rs exact check runs on the CPU in parallel; failures go to LP compaction or are discarded.
- **S4 (GPU).** Coarse Jacobi routing at 0.25 mm for the top K, several candidates per dispatch. Score: unrouted count, then overflow, then vias, then length.
- **S5 (GPU).** Coupling rounds for the top K′: history-field and tension forces → short pose relaxation with XPBD (keeps legality) → incremental reroute of affected nets. Keep a change only if the score improves.
- **S6 (GPU).** Fine routing at 0.1 mm inside corridors, with conservative clearance maps, for the top 1–2. Pour connectivity via connected components, then stitching vias.
- **S7 (CPU).** Raster → gridless polylines (collinear merge, 45° smoothing, pad-entry geometry) → exact clearance check → local fine-grid rip-up of violators → zone refill + KiCad DRC. kicad-cli's zone-refill option for DRC is unverified.

**Rough budget on a GTX 1650.** These are estimates, not measurements; memory-bound kernels are assumed to run at about 60% of 128 GB/s.

| Stage | 2-layer, 100 parts, ~150 nets, ~1k pads, 100×80 mm | 4-layer, 200 parts, ~350 nets, ~2.5k pads, 150×100 mm |
|---|---|---|
| S0 CPU prep | 0.2 s | 0.5 s |
| S1 GP (B=256, 600 its, successive halving) | 3–5 s (~16M density cells/it, ~6 ms/it early) | 4–8 s (B=128) |
| S2 DGR-lite (B′=64, 200 its; 64k / 240k GCells per replica) | 2–4 s | 5–8 s (~0.9 MB/replica × 3 buffers) |
| S3 legalize (B″=32, ~2k steps + guided search, exact check) | 2–3 s | 3–5 s |
| S4 coarse route 0.25 mm, top 16 / top 8 | 3–6 s (4 MB each, batched) | 10–20 s (16–256 MB each; 2 batches) |
| S5 coupling, 4–6 rounds, top 4 / top 3 | 5–10 s | 15–30 s |
| S6 fine route 0.1 mm, corridors, top 2 / top 1 | 3–8 s | 15–40 s (≤ ~0.5 GB with corridors) |
| S7 CPU cleanup + DRC + repair | 2–5 s | 5–15 s |
| **Total** | **~20–40 s** | **~1–2.5 min** |
| GPU share of wall time | ~80% | ~75–85% |

**How randomization is used.** Seeds and initial clusters; λ schedules, γ, halo scale, target density, routing-weight ramp; layer-direction preferences; Steiner topology choices; random rip-up subsets and net orders in the router; tempering temperatures. Each replica is one configuration. Many cheap runs are cut down to a few expensive ones, which fits the owner's "hundreds of runs, keep the best".

## 5. Main risks

- **Estimator fidelity.** DGR-lite or RUDY may rank placements poorly on 2-layer boards, where pin access and topology matter more than demand. Mitigate with the crossing term, the pin-access halos and the learned residual from idea 12, and gate on correlation (see §6).
- **Jacobi negotiated routing may oscillate or converge slowly.** Mitigate with random rip-up subsets, a present-cost ramp, and a serial CPU finish for the last few percent of nets.
- **wgpu engineering.** Hand-written FFT, adjoints, fixed-point scatters and buffer limits. Keep a PyTorch reference and diff-test every kernel against it.
- **Conflicting gradients.** Overflow and crossing gradients fight wirelength and density. You will need per-term normalization (the ePlace λ-ratio trick applied to each term) and schedules.
- **Replica collapse.** Needs niching.
- **Memory on 4-layer boards at 0.1 mm.** Corridors are mandatory.
- **Off-grid pads and odd pad shapes break raster DRC.** Needs the geometric pad-escape precompute plus the S7 repair; budget real effort here.

## 6. Cheap first experiment (PyTorch, about 1–2 weeks)

1. Export 5–10 real boards (2- and 4-layer, including a dense two-sided one) as JSON: footprints, poles, courtyards, pads, nets, outline.
2. In PyTorch, implement batched ePlace (B=256): WA + DCT density per side + poles + continuous θ with sin²(2θ). Add DGR-lite at 0.5 mm with soft-rasterized capacity and pin gradients, plus the crossing term.
3. Produce three arms of 64 placements each: (a) wirelength/density only, (b) + DGR-lite, (c) + DGR-lite + capacity from pinless bodies. Legalize with the **existing** annealing legalizer, and route with the **existing** Rust router.
4. Measure:
   - **Gate:** Spearman ρ between DGR-lite overflow and the real router's (unrouted connections, negotiation iterations). If ρ < 0.5, fix the estimator (or add the idea 12 CNN) before any wgpu work.
   - Fraction of placements that fully route, per arm.
   - Best-of-64 against the current serial best.
   - A synthetic check for pinless obstacles: a board with a large unconnected part blocking the only channel. Does arm (c) move it?
5. If the gate passes, port S1 and S2 to wgpu first, as the largest and most self-contained win. Port the router (S4–S6) second.

**Sources:**
- [Cypress (ISPD'25)](https://dl.acm.org/doi/10.1145/3698364.3705346), [NVlabs/Cypress](https://github.com/NVlabs/Cypress)
- [DGR (DAC'24)](https://dl.acm.org/doi/10.1145/3649329.3656530), [NVlabs DGR code](https://github.com/NVlabs/Differentiable-Global-Router)
- [RUPlace (DAC'25)](https://ieeexplore.ieee.org/document/11132838/)
- [Differentiable Net-Moving (DAC'25)](https://ieda.oscc.cc/res/papers/25-DAC25-DCGP.pdf)
- [Xplace](https://github.com/cuhk-eda/Xplace/blob/main/README.md), [GGR](https://dl.acm.org/doi/pdf/10.1145/3508352.3549474), [InstantGR](https://dl.acm.org/doi/10.1145/3676536.3676787), [GAMER](https://ieeexplore.ieee.org/abstract/document/9799536)
- [DATE'21 DL routability in DREAMPlace](https://www.cse.cuhk.edu.hk/~byu/papers/C112-DATE2021-DREAMPlace-Cong.pdf), [ABCDPlace](https://ieeexplore.ieee.org/document/8982049/), [DREAMPlace](https://github.com/limbo018/DREAMPlace)
- [Apollo](https://scopex-asu.github.io/files/publications/PD_ICCAD2025_Gu.pdf)
- [Package floorplanning with pin assignment (arXiv 2607.15005)](https://arxiv.org/abs/2607.15005)
- [ULTRA-2.5D (Gumbel orientation)](https://github.com/ElectroWarriors/ULTRA-2.5D)
- [ChipDiffusion](https://arxiv.org/abs/2407.12282)
- [jagua-rs](https://github.com/JeroenGar/jagua-rs), [sparrow paper](https://arxiv.org/abs/2509.13329)
- [XPBD](https://dl.acm.org/doi/10.1145/2994258.2994272)
