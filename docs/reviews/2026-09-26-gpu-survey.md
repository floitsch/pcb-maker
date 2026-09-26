# GPU survey: routing and placement where most of the work is data-parallel

2026-09-26. Second round of the [algorithm survey](2026-09-26-algorithm-survey.md),
this time constrained to algorithms that put most of the work on a GPU. Target
hardware is the one named in [gpu.md](../gpu.md): a GTX 1650 (4 GiB, Vulkan,
wgpu), so nothing here assumes CUDA or a large card. No GPU code was written;
four cheap CPU measurements that decide between the designs were run and are
reported at the end.

## How it was done

Six fresh agents without project history, each with a different lens. The
problem statement was the same for all (scale, rules, pours, the memory
limit, "many weaker randomized runs are acceptable"); what they were told
about our approach varied:

| Report | Lens | Given our approach? |
| --- | --- | --- |
| [1-blind](2026-09-26-gpu-survey/1-blind.md) | any field | no |
| [2-wavefront-parallel-sssp](2026-09-26-gpu-survey/2-wavefront-parallel-sssp.md) | parallel shortest paths, GPU routers | yes, incl. the failed Jacobi run |
| [3-fields-physics-fluid-idea](2026-09-26-gpu-survey/3-fields-physics-fluid-idea.md) | PDEs, physics; asked to evaluate the owner's fluid idea | yes, incl. the 2026-09-07 diffusion result |
| [4-populations-stochastic](2026-09-26-gpu-survey/4-populations-stochastic.md) | populations, sampling, parallel evaluation | yes |
| [5-differentiable-relaxations](2026-09-26-gpu-survey/5-differentiable-relaxations.md) | differentiable routing, LP relaxations | yes |
| [6-placement-joint](2026-09-26-gpu-survey/6-placement-joint.md) | GPU placement, joint place + route | yes, plus the memory numbers below |

## Short answer

Porting A\* does not help, and nobody proposed it. All six reports converge
on the same skeleton, which keeps what makes the current router good
(negotiated congestion) and replaces its serial core:

1. **Shortest paths as directional prefix-min scans** (GAMER, Lin/Liu/Young/
   Wong, ICCAD'21 / TCAD'23). Along a row with step costs w, one sweep is
   `d ← S + cummin(d − S)` (S = prefix sum), or equivalently a parallel scan
   over the tropical maps x ↦ min(x + a, b). Sweeping ±x, ±y, the diagonals
   and the layer axis until nothing changes gives *exactly* the Dijkstra
   field. Rounds ≈ bends + 1. Every net, window and replica is independent.
   All six reports put this (or its bit-parallel Lee variant) at the core.
   Verified here on the CPU: identical to Dijkstra on all test grids.
2. **All nets negotiate at once, but damped.** Plain Jacobi (everyone
   reroutes against one snapshot) is what failed here (Interf-U 448 s / 44
   vias against 81 s / 28). The reports agree on the fixes: reroute only a
   random 10–40 % of the conflicted nets per iteration, add a proximal
   "stay on your old path" discount, smooth the present cost, add small
   price noise; or keep Gauss–Seidel semantics with small batches of nets
   whose *corridors* (not bounding boxes) are disjoint, committing winners by
   priority (deterministic reservations). Which one is needed is an empirical
   question; the batch-size measurement below is the first data point.
3. **Clearance as fields.** Either per-class occupancy convolved with a disc
   of radius w_a/2 + clr + w_b/2 (linear, so K² convolutions per iteration for
   the whole board, own contribution subtracted), or a top-2-owner distance
   field (nearest and second-nearest copper owner per cell) that answers
   "is this cell legal for net n of width w" for every class at once. u32
   atomics only (WGSL has no float atomics): pack fixed-point distance and
   parent direction into one u32 and `atomicMin`.
4. **Coarse to fine.** A GPU global router on 0.4–1 mm tiles with capacities
   from the exact geometry, then fine routing only inside corridors (coarse
   route ± 1–2 tiles). This is what makes 0.05–0.1 mm affordable in 4 GiB
   and what makes corridor batches mostly disjoint.
5. **Pours on the GPU.** Connected-component labelling of the pour region per
   candidate (milliseconds), so island count becomes a score for every
   candidate; plus a resistor model of the pour (Laplace with pour pads as
   sources) whose current density marks the necks signals must not cut. This
   is where diffusion is the actual physics. Several reports independently.
6. **Raster to exact geometry.** String-pull against the distance field (step
   by the distance to the nearest obstacle), snap to 45°, relax polylines as
   elastic bands with signed-distance clearance (topology fixed), then the
   existing exact verifier and KiCad. This is also what lets the raster be
   coarser than today's 0.05 mm rungs.
7. **A funnel, not N full runs.** Batch hundreds of configurations at coarse
   resolution, keep the best by successive halving, run a few at full
   resolution. Replicas are the leading buffer dimension, so one dispatch
   covers all of them.

Estimated end-to-end times on the 1650 range from 10 s to 5 min for a
four-layer, 200-net board depending on the report and how many
configurations are screened (today: 5–20 minutes for one run). These are
memory-bandwidth estimates by the agents, not measurements.

## The fluid idea

Evaluated in report 3 and touched by reports 1, 4 and 5. Verdict: a literal
fluid is the wrong model, but each part of the idea maps onto something
known that runs well on a GPU.

- **Why literal fluids fail.** Immiscible fluids minimize interface length,
  so thin filaments retract into blobs; mass conservation makes colours fill
  area instead of forming wires; inertia only adds overshoot; one set of
  distributions per net does not fit in memory.
- **"Distance and length propagated through the field"** is the eikonal
  arrival-time field, i.e. item 1 above. **"Colour pushes toward its
  counterpart"** is descending the counterpart's arrival-time field.
- **Why our 2026-09-07 diffusion detoured.** A Laplace potential minimizes an
  L² energy, so current spreads over parallel paths; wires want L¹ (length).
  The fix is a leak to ground: with a screened potential, −ε·log u approaches
  the geodesic distance (Varadhan). **Checked here:** on the wall fixture,
  4-neighbour, the plain field gives 42 cells, the screened field gives the
  shortest 28 at leak 0.1 and 0.01 (34 at 0.001, where screening is too
  weak). So the earlier conclusion "fields are only useful as prices" holds
  for static diffusion, not for fields in general. In log domain the screened
  solve becomes a soft-min Bellman–Ford, i.e. the eikonal sweeps again, which
  is why item 1 dominates it in practice.
- **"Strengthen finished connections"** is Physarum / dynamic
  Monge–Kantorovich: conductance grows with flux (μ̇ = |μ∇u|^β − μ). For β = 1
  it provably converges to shortest paths (Bonifaci, Mehlhorn, Varma 2012),
  for β > 1 to branched trees; saturating conductance is a natural lock.
  Slow (hundreds of Poisson solves), so only at coarse resolution, for
  multi-pin topology.
- **"Colours competing"** has a published form: Lonardi, Baptista, De Bacco,
  *Immiscible color flows in optimal transport networks* (Front. Phys. 2023),
  where colours share capacity. For PCB the sharing must become exclusion
  with clearance, and the price of that exclusion is PathFinder's congestion
  price. Continuum theory (Beckmann, Wardrop, Carlier–Jimenez–Santambrogio
  congested transport) says those prices are the right dual variables, so
  the field view refines negotiated routing rather than replacing it.
- **The soft version is the useful one.** Replace min by a soft-min in the
  sweeps; the gradient of each net's soft distance with respect to cell cost
  is its expected cell usage, a smooth "colour density". Summed and priced,
  that is entropic multicommodity flow (stochastic user equilibrium); it
  gives smooth congestion maps for the placer and can be *sampled* to get
  hundreds of distinct discrete routings (reports 1, 3, 5).
- **Colour territories with a memory cost independent of the net count**
  (report 3): one label and one distance per cell for all nets, first
  arrival claims the cell, nets whose pins end up in separate pieces of their
  territory raise their bid (auction dynamics). Each net routes inside its
  territory with half the clearance to the border, so territories are
  legal by construction and the pour is the unclaimed background. Cheap
  diversity; territories waste area, so it is a coordinator and initializer,
  not a router.
- **Vias** in all field variants are a coupling along the layer axis: a
  vertical relaxation step (or vertical conductance) at via-legal cells,
  priced by the via cost; via legality is one extra column mask.

## Does quantity beat quality?

Report 4 makes the argument that matters for the "many weaker runs" premise:
for roughly normal final scores, best-of-N improves on the mean by about
σ·√(2 ln N): 3.0σ at N = 100, 3.7σ at 1000. That is slow. It pays when
combined with **decomposition or recombination**: choose per region or per
net group from different replicas (windowed large-neighbourhood search that
commits disjoint windows, net-level partition crossover between parents,
path selection over a pool of candidates), which searches N^R combinations.
The main risk is correlated failure: when a BGA escape or a connector
fan-out is infeasible under the placement, every replica fails in the same
cells and the fix belongs to placement. Report 4 also proposes sharing the
learned history across replicas (ant-colony style) so parallel replicas keep
negotiation's convergence.

What the measurements below say about our router is in the experiments
section.

## Placement on the GPU, and together with routing

Report 6 with contributions from 1, 4 and 5.

- **Prior art to read first: Cypress** (NVIDIA, ISPD'25 best paper,
  [NVlabs/Cypress](https://github.com/NVlabs/Cypress), Apache-2.0), a GPU PCB
  placer built on DREAMPlace with per-side density, rotation and a smoothed
  net-crossing cost. It is the closest existing system to what we would
  build.
- **One board cannot fill a GPU; a batch can.** 256 replicas × 250 bodies is
  ISPD-benchmark scale. All state gets a leading replica dimension:
  seeds, λ schedules, target density, side assignment differ per replica.
- **Bodies as poles** (inscribed discs, as in the sparrow/jagua-rs nesting
  work): arbitrary rotation costs nothing, density becomes a disc splat,
  torque comes from field samples at the pole centres, overlap is
  circle–circle penetration. Exact polygons only for the final check.
- **The pinless-obstacle fix:** make routing capacity a soft-rasterized,
  differentiable function of every body's pose (sigmoid of the body's
  signed distance). Congestion then pushes all bodies, connected or not.
- **Joint place + route through distance fields:** by the envelope theorem
  the derivative of a net's routed cost with respect to a pin position is the
  gradient of the net's price-weighted distance field at the pin. The fields
  exist anyway from the router, so ePlace gets a routability force with no
  extra routing (reports 5, 6; report 3 of the first round proposed the same
  from transportation theory). The router's history raster used as a
  potential on every body replaces today's "nudge congested parts".
- **Crossings on two layers:** a smoothed ratsnest crossing count (as in
  Cypress) is a better proxy than RUDY on two-layer pour boards.
- **Side choice** as a small area pre-solve (per side, SMD area plus
  through-hole parts on both sides) that detects infeasible dense two-sided
  boards before placement, then flip moves in parallel tempering.
- **Legalization:** XPBD rigid bodies with SDF contacts on the GPU, then
  sparrow-style guided local search with batched candidate evaluation, then
  jagua-rs as the exact check (Rust, drop-in).
- Diffusion-model placers (ChipDiffusion) and RL: all reports say skip for
  now; there is little PCB data and guided sampling needs the same
  objectives as above.

Report 6 estimates 20–40 s for a two-layer 100-part board and 1–2.5 min for
a four-layer 200-part board, place and route, 75–85 % of wall time on the
GPU.

## Memory, measured

Per-net fields over each net's bounding box plus 2 mm, one f32 per cell and
copper layer, summed over all nets (script:
`experiments/algorithm-exploration/net_window_memory.py`):

| Board | Layers | Nets | Σ windows / board area | MB at 0.1 mm | MB at 0.05 mm |
| --- | ---: | ---: | ---: | ---: | ---: |
| ecc83 | 2 | 9 | 1.3 | 2 | 10 |
| hierarchy | 2 | 50 | 2.8 | 18 | 71 |
| pic | 2 | 34 | 2.7 | 34 | 137 |
| interf-u | 2 | 110 | 6.3 | 63 | 251 |
| esp32-c3 dut | 2 | 42 | 4.2 | 12 | 49 |
| dual-esp32 | 2 | 52 | 6.0 | 19 | 75 |
| sonde-xilinx | 2 | 26 | 2.5 | 7 | 28 |
| multichannel | 2 | 79 | 2.5 | 24 | 96 |
| stickhub | 2 | 45 | 6.9 | 4 | 15 |
| coldfire | 4 | 209 | 11.8 | 273 | 1090 |
| openair-max | 4 | 120 | 8.6 | 99 | 396 |
| tiny_tapeout | 4 | 108 | 19.4 | 262 | 1049 |
| video | 4 | 371 | 30.1 | 1603 | 6412 |

Two-layer boards fit dozens of replicas at 0.1 mm. Four-layer boards need
f16 or u16 fields, corridors instead of bounding boxes (report 6 estimates a
3–10× cut), or both; the video board at 0.05 mm does not fit without them.

## Where the reports disagree or are unsure

- **Jacobi or Gauss–Seidel.** Reports 1, 3, 5 and 6 expect damped Jacobi
  (random subsets, stickiness) to converge within 1.5–2× of serial
  iterations. Report 2 doubts it for the endgame and prefers small
  Gauss–Seidel batches with ordered commits plus many replicas. Our own
  Jacobi run supports the caution; the batch-size measurement below is the
  first curve.
- **Resolution.** Report 5 argues for 0.1–0.125 mm plus continuous
  legalization; report 3 says the h/√2 rasterization margin eats too much of
  a 0.2 mm clearance at 0.125 mm and wants ≤ 0.07 mm near dense pins.
- **Round counts.** Sweep rounds grow with bends; U-shaped obstacles in large
  windows are the bad case. Report 2 runs sweeps inside 16×16 tiles
  scheduled asynchronously (tile-level Bellman–Ford / near–far) for that
  reason. Measured here: 3–8 rounds with 4 neighbours and 9–24 with 8 on
  PCB-like grids up to 512×512.
- **wgpu practicalities** all reports raise: no float atomics, 128 MiB
  default binding size (request the adapter limit), 16 KiB default workgroup
  storage, no grid-wide barrier (use indirect dispatch with a GPU-side
  scheduler), no FFT library, per-dispatch overhead of 10–50 µs, which is why
  replicas must share dispatches.

## Experiments run for this survey

All CPU, on this container (4 cores), boards from the corpus without native
KiCad (`skip_native_verification`), one Rayon thread per run.

EXPERIMENTS_PLACEHOLDER

## Suggested order

1. Finish the two CPU measurements that decide the architecture (above):
   how much seeds spread and where the failures sit; how quality falls with
   batch size, with and without damping.
2. Validate the primitive on the GPU: a wgpu sweep kernel (tile-local scans,
   packed u32 atomicMin) for single nets, then 128 concurrent windows, against
   the CPU A\* on windows cut from real boards. Go/no-go: at least 20× one
   CPU core in throughput (report 4's threshold).
3. Occupancy/clearance fields and one full single-replica GPU negotiation
   loop at 0.1 mm on two-layer boards, compared with the serial router.
4. Coarse global routing on tiles with replicas and successive halving.
5. Batched placement (start from Cypress's cost functions) with the
   distance-field routability force.
6. Pour CCL and resistor pricing; raster-to-geometry legalization.
