# GPU grid A* prototype (2026-10-21): no-go

The crate and the router dump patch live on branch gpu-astar-prototype
(commits ca9095d and 47a784b on 0d13e80), not on main. This page is its
README with the measurements.

# pcb-gpu-proto: the router's lattice A* on a GPU (go/no-go, 2026-10-21)

A bounded experiment, not integrated anywhere. It answers whether a wgpu
compute backend for the router's search could gain more than 20 % end to
end on budget-bound boards with byte-identical routes. **Answer: no-go**,
for the reasons below; what would have to change first is listed at the end.

## What is here

- `src/dump.rs`: reads the search records the router writes with
  `PCB_GPU_DUMP` (a patch to crates/pcb-router, `router/gpu_dump.rs`, kept
  on the branch `gpu-astar-prototype` only). One record is one negotiation
  search with everything it reads resolved for its net (class and neck
  zones, own stamps masked, own via rings, the pour cover and thermal
  guards as flags, the via step's cost and target layers per cell, the
  goals, the corridor, the heuristic's points or coarse map) over the
  region it can touch (the expanded nodes and one around, inside the
  window), plus the router's path and expansion count.
- `src/reference.rs`: the CPU reference, the router's search
  (router/search.rs, not hard, not the clean-up) on those arrays: the same
  step and via costs in the same floating point order, the same heuristic,
  the same keys (the estimate's bits above the state; region-local states
  order as the global ones) in a copy of the router's four-way key heap.
- `src/astar.wgsl`, `src/gpu.rs`: the compute kernel, one search per
  workgroup of 32 invocations, many searches per launch. Invocation 0 pops
  (the four-way heap, its top 1365 entries in workgroup memory, the rest
  in global memory); invocations 0-7 evaluate the planar steps and 8.. the
  via's target layers of the popped node at once (distinct targets, no
  conflicts); invocation 0 pushes what improved. Dense per-search node
  arrays (cost, parent, seen, closed: 8 bytes a state) over the region.
  Products that feed an addition pass through an exclusive or with a zero
  the compiler cannot see, so no driver contracts them into fused
  multiply-adds (`--contract` drops these guards).
- `src/latency.rs`: a pointer chase, one invocation: what a dependent
  global read costs.
- `scripts/shares.py`: expansions by batch kind from `PCB_GPU_STATS`.

```sh
# on the branch: dump iteration 1 of bench-search (3 iterations) on a v16 placement
P=build/github-layout-v16/wntrblm__Castor_and_Pollux__mainboard/layout/placed
RAYON_NUM_THREADS=4 PCB_ROUTER_NO_WALL_GUARD=1 PCB_GPU_STATS=castor-stats.csv \
  PCB_GPU_DUMP=dumps/castor PCB_GPU_DUMP_ITERATION=1 \
  target/release/pcb-maker bench-search $P mainboard 3 auto
cargo run --release -p pcb-gpu-proto -- verify dumps/castor      # reference against the router
cargo run --release -p pcb-gpu-proto -- gpu dumps/castor         # GPU at 1, 8, 64, 256 per launch
cargo run --release -p pcb-gpu-proto -- lookahead dumps/castor   # parallelism inside one search
cargo run --release -p pcb-gpu-proto -- latency
python3 crates/pcb-gpu-proto/scripts/shares.py castor-stats.csv
```

Dumps are 0.8-2.8 GB a board (they lived in the worktree's target/, not
committed).

## Device

NVIDIA GeForce GTX 1650 (TU117, 14 SMs, 4 GB), driver 615.78.08, Vulkan
1.4.351 through wgpu 30.0.1 (Vulkan backend); 2976 MiB free at the start
(the desktop holds about 750 MiB); max storage binding 2147 MB, 48 KB
workgroup memory. Clocks at P0 1965 MHz during the runs. The CPU side: the
5950X under the other agents' load (load average 11-20), so CPU rates
swing by 15 % between runs.

Data: Castor (4 layers, 568 x 865 nodes), k30-SBC (4 layers, 1261 x 789)
and jetson-nano (8 layers, 791 x 435), route mode's lowering of the v16
placements (`bench-search ... auto`, 3 negotiation iterations), every
negotiation search of iteration 1: 273, 398 and 745 searches, 1.8 M, 11.9 M
and 11.2 M expansions.

## 1. Determinism

| board | searches | CPU reference = router (path and expansions) | GPU = reference, every launch size and the negotiation's batches | GPU without the FMA guards |
|---|---|---|---|---|
| Castor | 273 | 273 | 273 | 273 |
| k30-SBC | 398 | 398 | 398 | 398 |
| jetson | 745 | 745 | 745 | 745 |

All 1416 searches byte-identical three ways: the same path node for node
and the same number of expansions (stale pops not counted, as the router
counts). NVIDIA's compiler did not contract this code even without the
guards (all identical with `--contract` too); what the guards cost is
within the noise (k30 at 256 a launch 3.03 M expansions a second with
them, 3.68 M without; jetson 2.63 M and 2.33 M) and they stay: Metal
compiles with fast math unless told otherwise, other drivers may fuse.

## 2. Expansions per second

| | Castor | k30-SBC | jetson |
|---|---|---|---|
| router, route mode, `profile rate` (x1184, all phases, 4 threads) | 3.58 M | 3.03 M | 3.64 M |
| router inside `search` (bench, these searches, 4 threads side by side) | 3.38 M | 3.33 M | 2.69 M |
| CPU reference, one thread, these searches | 3.4-4.8 M | 3.5-4.2 M | 3.4-4.7 M |
| GPU, 1 search a launch | 0.21 M | 0.18 M | 0.20 M |
| GPU, 8 a launch | 0.51 M | 0.67 M | 0.51 M |
| GPU, 64 a launch | 1.69 M | 2.42 M | 2.36 M |
| GPU, 256 a launch (fewer where 1.5 GB is reached) | 2.41-3.98 M | 3.03 M | 2.63 M |
| GPU, the negotiation's own batches (parallel batches side by side, a net's searches in rounds) | 0.22 M | 0.18 M | 0.22 M |

The reference is faster than the router's search because it reads
pre-resolved arrays (no neck classes, no own-stamp bitset, no coarse map to
build). One GPU search runs at 4.5-5.5 microseconds an expansion (0.6
stale pops each besides): its critical path is a chain of dependent global
reads (the pop's node and word, the heap's deeper levels, the neighbours'
records, the coarse heuristic, the pushes' parents), and a pointer chase
on this card costs 47 ns a read in 16 KB, 146 ns in 256 KB, 236 ns in 4 MB,
255 ns in 256 MB: about 17-22 round trips an expansion. One warp per search
hides none of it; the heap's top levels in workgroup memory (1, 1365 or
5461 entries) changed the single-search rate by nothing measurable, and
naga's bounds checks cost about 10 %. Many searches per launch hide the
latency of each other, but a launch lasts as long as its longest search
(Castor's 66 000-expansion search alone takes 0.3 s), so 256 a launch
reaches 0.6-0.9 of one CPU core, not more.

## 3. Launch overhead and memory

| searches a launch | kernel (submit to done), empty searches | upload | download |
|---|---|---|---|
| 1 | 0.05-0.07 ms | 0.07-0.10 ms | 0.04-0.05 ms |
| 8 | 0.05-0.07 ms | 0.07-0.09 ms | 0.04-0.05 ms |
| 64 | 0.05-0.06 ms | 0.07-0.09 ms | 0.04-0.05 ms |
| 256 | 0.06-0.07 ms | 0.09-0.12 ms | 0.04-0.05 ms |

The prototype uploads every search's resolved inputs per launch (0.4-1.9 s
per board at one a launch, 1-4.7 s at 256; a production backend would keep
the maps on the device). Per search, 8 bytes a state for the node array
plus the heap:

| | Castor | k30-SBC | jetson |
|---|---|---|---|
| window states (median = max: the whole board; corridors confine the search, not the window) | 1.97 M (15.7 MB) | 3.98 M (31.8 MB) | 2.75 M (22.0 MB) |
| states the search touched (dumped region), median / max | 52 k (0.4 MB) / 1.37 M (10.9 MB) | 222 k (1.8 MB) / 2.5 M (20.0 MB) | 58 k (0.5 MB) / 2.0 M (16.1 MB) |
| largest open list, median / max entries (8 bytes each) | 7.6 k / 136 k | 24.7 k / 84 k | 10.9 k / 155 k |
| device memory at 256 a launch (inputs, node arrays, heaps) | 1012 MB | 1749 MB | 1774 MB |

Dense node arrays over the window would need 4-8 GB for 256 searches:
more than this card has. Paged (tile) or hashed node storage would be
required.

## 4. What share a GPU could take

The negotiation routes nets in batches of disjoint windows; a batch of one
runs on one thread, and results are identical only if the batches stay as
they are. From the router's profile lines (route mode, x1184, v16
placements):

| board | single-net batches | parallel batches | nets per parallel batch | parallel share of batch time | parallel batches' wall share of the run |
|---|---|---|---|---|---|
| Castor | 2589 (121.6 s) | 600 (5.4 s of net time, 3.9 s wall) | 2.2 | 4.3 % | 1.6 % of 242 s |
| k30-SBC | 3146 (187.6 s) | 234 (4.8 s, 3.4 s wall) | 2.3 | 2.5 % | 0.8 % of 416 s |
| jetson (5 routers) | 15 210 (615.3 s) | 2576 (44.4 s, 35.0 s wall) | 2.1 | 6.7 % | 3.9 % of 896 s |

In the first three iterations (`PCB_GPU_STATS`, bench-search) the share
is higher, 23 %, 9 % and 34 % of the negotiation's expansions, with 2.5,
2.6 and 2.2 nets per parallel batch (at most 5, 7 and 6); later iterations
widen the windows and the clean-up and via reduction (single searches by
construction) take most of the run. So the GPU could take at most 1-4 %
of the wall clock without changing results, and it would take it in
launches of two or three searches, where it runs at 0.2-0.7 M expansions a
second against the CPU's 3-4 M: slower, not faster.

Can one search be split across workgroups, the papers' model, with the
router's order? GA* (Zhou and Zeng) pops thousands of queues at once and
returns an optimal path for an admissible heuristic but expands other and
more states, in a nondeterministic number (they report 20 % swings) and
with other ties; BGPQ (Chen et al.) and the bucket queue of Al Khansa et
al. (ICS 2026, bidirectional) pop a batch of the best keys and expand them
together, again a relaxed order. Our heuristic is weighted (inadmissible)
and closed nodes are never reopened, so any other expansion order gives
other paths. An exact variant exists: pop the next K keys, expand them
speculatively side by side (a step's cost depends only on static data and
the expanded node's cost and parent), then commit in key order, re-testing
closed and cost per target, and cut the round at the first popped key that
a key pushed by an earlier member of the round undercuts (or whose node an
earlier member improved). Measured on the reference (`lookahead`): the next
pops already queued in order at an expansion number 50-81 on average, but
a round that starts where the last was cut commits only

| window cap K | 8 | 16 | 32 | 64 | 128 |
|---|---|---|---|---|---|
| Castor, expansions per round | 5.4 | 7.6 | 9.3 | 10.2 | 10.5 |
| k30-SBC | 6.3 | 9.8 | 13.5 | 16.4 | 17.7 |
| jetson | 5.6 | 7.8 | 9.7 | 10.7 | 11.2 |

A round needs an exact batched delete-K and insert of about 3.5 K keys
(a BGPQ-like heap), K parallel expansions and an ordered commit: ten or more
dependent global round trips, about 2.5-3 microseconds on this card, for
10-17 expansions: 3-6 M a second, CPU parity at best.

## Go / no-go

**No-go.** The bar was a projected end-to-end gain above 20 % on
budget-bound boards at identical results, with the CPU reference kept. The
exact GPU search exists and is identical on all 1416 searches, but:

1. Only the parallel batches can move to the GPU without changing results:
   1-4 % of the wall clock, in batches of about two nets. Amdahl caps the
   gain at that even for an infinitely fast GPU, and at two searches a
   launch the GPU is 8-20 times slower than a CPU core (0.18-0.22 M
   expansions a second on the negotiation's own batches).
2. One GPU search is about 20 times slower than one CPU search (latency
   bound: 17-22 dependent global reads an expansion at 150-255 ns each).
   Only hundreds of concurrent searches make up for it, and then only to
   0.6-0.9 of one core on this card.
3. Splitting one search with the router's order is possible (speculative
   rounds with an ordered commit) but yields 10-17 expansions a round, CPU
   parity on this card at best, behind an exact batched GPU heap that does
   not exist yet.

## What would have to change first

- The negotiation would have to offer hundreds of independent searches per
  synchronisation point at identical results. Today it offers about two.
  The two ways tried, Jacobi batches (every net against one snapshot) and
  speculative batches (kept where nothing read had changed), converged
  worse or kept too few routes (docs/router.md, "Known limits") and were
  removed on 2026-10-16. The parked corridor graph (nets assigned disjoint
  corridors first) is the only candidate left; it is a router change with
  its own quality question, decided on the CPU first.
- Or one search would have to be split: an exact batched priority queue on
  the device (delete the K least keys, insert a batch, in the router's key
  order), parallel ordered commit (per target the least total, ties to the
  earliest member, a prefix minimum of the pushed keys for the cut), and a
  device whose dependent-read latency is well below 150 ns. The
  speculative-round idea can be measured on the CPU first (threads per
  round are too slow; it is a question of SIMD and prefetching).
- Per-search memory: node arrays over the touched region, not the window
  (tile pages allocated on first touch, or a hash), since the windows are
  the whole board.

A production backend would also need: buffer lifetimes across iterations
(static maps uploaded once per lowering and rule class; occupancy, history
and the hot records resident and updated in place; node arrays pooled and
reset by a generation tag instead of cleared); incremental occupancy
updates (the stamp diff as a scatter kernel after each batch, and a
device-host synchronisation after every single-net batch, which restamps
before the next net); the own-stamp mask and own via rings per search; the
neck classes per target node; the coarse distance transform per search on
the device; the pour model's reads (layer cut, cover, guards, the via
cover word, the plane-target masks; the fill probe stays on the CPU);
multi-layer vias (the free-layer masks, via cost per cell, up to 24 layers
in this layout); the hard and clean-up cost variants; heap capacity without
the reference's hint (growth or a bound); the driver's watchdog for long
launches; and the determinism rules: no fast math, the FMA guards, no
flushed denormals, the CPU path as reference and fallback without an
adapter.
