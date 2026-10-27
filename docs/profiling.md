# Profiling the router and the layout

How to take a CPU profile of a route, where the route's CPU goes, and what
the profile round of 2026-10-25 changed (branch router-profile, snapshots
x14380 and x14381 against main's x1820, 23f629e). The layout round of the
same day ("The layout round" below: branch layout-profile, snapshot x14490
against main's x1824, 09299b6) profiled everything else a layout does.
The memory round of 2026-10-26 ("Memory" at the end: branch
memory-round-3, snapshot x14553 against main's x1830, a905905) profiled
where a route's peak resident size goes. The second router round of
2026-10-27 ("The second router round" below: branch router-profile-2,
snapshot x14967 against main's x1851, 71953b0) profiled the search again
after both.

## How to profile

perf is not installed on the benchmark machine and there is no root.
The Arch package runs from any directory, as the user:

```sh
mkdir perf && cd perf
curl -O https://geo.mirror.pkgbuild.com/extra/os/x86_64/perf-7.2.11-1-x86_64.pkg.tar.zst
curl -O https://geo.mirror.pkgbuild.com/extra/os/x86_64/libpfm-4.14.1-1-x86_64.pkg.tar.zst
mkdir root && for p in *.zst; do tar -C root -xf $p; done
LD_LIBRARY_PATH=$PWD/root/usr/lib root/usr/bin/perf --version
```

`kernel.perf_event_paranoid` is 2, which allows sampling one's own
processes in user space (`cycles:u`), all a route needs. Several
`perf record` sessions at once exhaust the locked-memory allowance
(`Permission error mapping pages`): pass `-m 16`.

The release profile has no debug information and no frame pointers.
Build a profiling binary into its own target directory (it shares
nothing with `target/` and costs a full build, 2.5 minutes):

```sh
CARGO_TARGET_DIR=target-prof RUSTFLAGS="-C force-frame-pointers=yes" \
  CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release
```

Route one placement directly (the binary registers with the memory
ledger itself), at pinned rates and without the wall guard so that the
run is the same as any other's, pinned to two cores:

```sh
PCB_WORK_RATE=3.15e6,150e6 PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1 RAYON_NUM_THREADS=2 \
  taskset -c 8,9 perf record -F 499 -m 16 -g -o castor.data -- \
  target-prof/release/pcb-maker route-kicad-board \
  build/github-layout-v16/wntrblm__Castor_and_Pollux__mainboard/layout/placed mainboard \
  out/route router.json --reference none          # router.json: {"ladder_budget_seconds": 300}
perf report -i castor.data --no-children --sort symbol --stdio -g none
perf report -i castor.data --no-children --sort srcline --stdio -g none \
  --symbols='<pcb_router::router::Router>::search'
```

The search is one function with everything inlined into it, so its
self time (76 to 90 %) says little; `--sort srcline` restricted to it
splits it by source line (a minute per profile; the "could not read first
record" lines of perf's addr2line are harmless). perf report finds the
binary by the path it recorded: keep the profiled binary there until
every report is written (a rebuild in between gives wrong lines; copy it
aside before building again). Record into a directory on disk, not
/tmp: a profile is 20 to 85 MB, and a full quota ends the recording
(`failed to write perf data`) with an unreadable file.

**CPU measurements** pair two snapshots on the same physical cores: one
binary on two logical CPUs (8, 9), the other on their SMT siblings (24,
25), started together, so that both see the same core, cache and memory
traffic of the other jobs; then the same with the binaries swapped. The
pair's CPU seconds are the router process's own (utime + stime from
/proc, polled; kicad-cli's checks, its children, left out), beside
`perf stat -e cycles:u,instructions:u` of the process tree. Pairs on
separate physical cores (0, 1 against 2, 3) differed by up to 6 % between
the two orientations on this machine under load; the siblings did not.

## Where the route's CPU goes (main, 2026-10-25)

Route mode, one run per board under perf (x1820's code, 23f629e),
two search threads pinned to two cores: the fingerprints (Castor,
k30-SBC, jetson, Sisu; v16 placements, a quarter budget: 300 s of work)
and two dense boards at the task budget (SmartSpin2k, protoesp; v19
placements, 1200 s), where the window searches take half the work.

**The top symbols by self time** (% of the route's samples; in brackets
where the symbol is not among the board's top 15):

| symbol (self) | Castor | k30-SBC | jetson | Sisu | SmartSpin2k | protoesp |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `Router::search` | 76.03 | 80.15 | 81.82 | 77.96 | 88.97 | 90.40 |
| `Router::search::{closure#2}` | 5.70 | 6.56 | 5.09 | 4.74 | 3.89 | 4.59 |
| `Router::stamp` | 4.40 | 3.07 | 2.25 | 5.16 | 1.40 | 1.79 |
| `Router::plan_corridor` | 1.20 | 2.54 | 2.54 | 2.23 | 0.55 | 0.37 |
| `Router::plan_corridor::{closure#2}` | 0.74 | 1.87 | 1.93 | 1.55 | 0.40 | 0.26 |
| `Router::plan_corridor::{closure#3}` | 0.57 | 1.15 | 1.33 | 1.03 | 0.21 | 0.13 |
| `PourMap::build` | 2.64 | 0.67 | (0.08) | 0.61 | - | 0.12 |
| `Shape::distance_to_point` | 1.10 | 0.20 | 0.16 | 0.86 | - | 0.07 |
| `StateMap insert (hashbrown)` | 0.25 | 0.22 | 0.74 | 0.55 | (0.07) | 0.12 |
| `geometry::point_in_polygon` | 0.44 | 0.11 | 0.21 | 0.71 | - | 0.12 |
| `Router::analyze_pours_uncounted` (an iterator closure) | 0.86 | 0.19 | - | (0.17) | - | - |
| `ceil` | (0.21) | 0.32 | 0.17 | (0.12) | 0.25 | 0.31 |
| `Router::mask_own` | (0.14) | (0.07) | 0.14 | 0.24 | 0.13 | 0.14 |
| `Router::branch_conflicts` | - | - | 0.16 | (0.09) | 0.38 | 0.06 |
| `fill::Index init (OnceCell)` | 0.29 | 0.18 | - | - | - | - |
| `FillProbe::blocker` | 0.45 | - | - | - | - | - |
| `PourMap::pieces_near` | - | - | - | 0.44 | - | - |
| `verify::verify_with_rules` | - | - | 0.40 | - | - | - |
| `Router::kicad_spokes` | 0.39 | (0.07) | - | - | - | - |
| `geometry::segment_segment_distance` | - | - | 0.15 | 0.20 | - | - |
| `PourMap::piece_near` | - | - | - | 0.31 | - | - |
| `HashSet<usize> insert (SipHash)` | 0.23 | - | - | - | - | - |
| `Router::route_net` | (0.08) | 0.09 | 0.14 | (0.14) | - | - |
| `Router::unstamp` | (0.10) | - | (0.07) | (0.11) | (0.07) | 0.23 |
| `hypot` | - | - | - | - | 0.22 | - |
| `Router::pour_report` | (0.15) | - | - | 0.18 | - | - |
| `__divti3` | - | - | - | - | 0.15 | - |
| `llroundf32x` | - | - | - | - | 0.13 | - |
| (an unresolved address) | - | - | - | - | 0.12 | - |
| `placer Grid::query` | (0.11) | 0.11 | (0.07) | (0.07) | - | - |
| `malloc` | - | - | - | - | 0.11 | - |
| `Shape::aabb` | (0.06) | - | - | (0.09) | 0.10 | - |

The search is 76 to 90 % of every route; with its heuristic (the
closure, 4 to 7 %) 81 to 95 %. Beside it: the occupancy updates
(`Router::stamp`, 1.4 to 5.2 %), the tile planner (`plan_corridor` and its
closures, 0.8 to 5.8 %: the fingerprints' negotiation plans corridors for
every net, the dense boards' searches are long), the pour maps and
analyses on the pour boards (Castor 6.5 %, Sisu 3.3 %). Hashing is
0.1 to 0.7 % (the targets' `StateMap`), allocation at most 0.11 %
(`malloc` on SmartSpin2k), memcpy and memset below 0.05 %, the corridor instrumentation (`note_search`, a few atomics a
search) below the listing's 0.05 %.

**The hot spots** (the search's source lines grouped, % of all samples):

| hot spot | Castor | k30-SBC | jetson | Sisu | SmartSpin2k | protoesp |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| heap (push, pop) | 14.8 | 16.2 | 17.4 | 16.0 | 18.2 | 17.9 |
| neighbour steps (node, clearance, cost) | 10.8 | 12.8 | 13.9 | 12.9 | 13.1 | 14.8 |
| search, lines below 0.3 % | 12.0 | 10.4 | 9.9 | 10.3 | 10.1 | 10.4 |
| inlined std (indexing, iterators) | 9.6 | 9.7 | 9.1 | 10.0 | 9.0 | 10.6 |
| occupancy lookups | 6.7 | 8.1 | 8.6 | 8.1 | 11.4 | 10.4 |
| unattributed (search.rs:0, spills) | 7.5 | 6.6 | 8.7 | 8.0 | 8.7 | 8.4 |
| hop test (hops_from_a_via) | 6.5 | 5.8 | 3.8 | 3.3 | 8.3 | 8.9 |
| heuristic (search closure 2) | 5.7 | 6.6 | 5.1 | 4.7 | 3.9 | 4.6 |
| via steps | 3.9 | 6.2 | 5.3 | 4.8 | 3.3 | 3.0 |
| tile planner (plan_corridor) | 2.5 | 5.6 | 5.8 | 4.8 | 1.2 | 0.8 |
| static maps (edges, vias, layers) | 3.0 | 2.8 | 3.2 | 2.9 | 4.4 | 3.4 |
| stamping (Router::stamp) | 4.4 | 3.1 | 2.2 | 5.2 | 1.4 | 1.8 |
| pours (maps, analyses, spokes) | 6.5 | 1.4 | 0.5 | 3.3 | 0.0 | 0.3 |
| pop and expansion set-up | 1.3 | 1.5 | 1.9 | 1.7 | 2.5 | 2.4 |
| hashing (hash maps) | 0.5 | 0.2 | 0.7 | 0.6 | 0.1 | 0.1 |
| allocation, memcpy, memset | 0.0 | 0.0 | 0.0 | 0.0 | 0.1 | 0.0 |

- **The heap** (15 to 18 %): almost all of it one load, the four children
  of a sift's level (`heap.rs:90`, 8 to 11 % alone): a pop moved the
  heap's last key to the root and sifted it down through every level, a
  dependent cache miss a level.
- **The neighbour steps** (11 to 15 %) and **the occupancy lookups** (7 to
  11 %): the search node of each neighbour (`closed`, line 342) and its
  occupancy byte (`occupancy.rs:42`) a row of the lattice away, on other
  cache lines; the indexing and the unattributed lines (`search.rs:0`,
  register spills) are a further 16 to 19 %, mostly the same steps.
- **The hop test** (3.3 to 8.9 %): `hops_from_a_via` walked back up to
  `ceil(reach / pitch) + 1` nodes along the path (reach the via drill
  plus the hole-to-hole spacing: 6 to 12 steps) for every expansion that
  may place a via, two divisions a step and the parent of each node a
  dependent load.
- **The heuristic** (3.9 to 6.6 %): with up to 12 target points, a serial
  minimum over the points for every push.
- **The via steps** (3 to 6 %): the other layers' search nodes at the
  cell (line 532).

## The changes (bit-identical)

Three hot spots with the best ratio of gain to risk, each identical in
its result by construction:

1. **The hop test walks only where a via can be** (`since_via`). A
   search node now records how many steps along its path it lies from the
   last node arrived at through a via or from a source (the byte the
   12-byte `SearchNode` had as padding; a compile-time assertion keeps the
   size), set wherever its parent is. The walk back finds a via only
   within `since_via` steps, so where `since_via` exceeds the steps the
   walk takes, its answer is "no hop" without walking; elsewhere it
   walks as before, by coordinates (no division a step) and with its
   reach computed once a search. The parent chain of an expanded node
   consists of closed nodes, whose parents no longer change, so the count
   is that of the chain the walk follows.
2. **The heap's root waits for the next push** (`KeyHeap`). A pop leaves
   the root empty and the next push fills it, sifting the new key down
   from the root; a pop with the root still empty fills it as a plain pop
   does. The search pushes a node's neighbours right after popping it,
   and their estimates are near the least: they stop a level or two
   below the root, where the heap's last key sank through every level.
   The sift also prefetches the next level's children (128 bytes) while
   it compares this level's. Keys order totally (the estimate's bits
   above the state), so any correct heap pops the same keys: the three
   heap tests compare it with `BinaryHeap` (one new: a search's rhythm of
   pops and pushes, pops of an empty heap).
3. **The expansion prefetches its neighbourhood**: the search nodes and
   packed maps (`Hot`) of the rows below and above, their occupancy bytes,
   and the node's own cell on the other layers, before the steps read
   them (prefetches without bounds checks: a prefetch faults on no
   address). A hint only: it changes no value.

Not taken: the heuristic (a serial minimum of at most 12 points; split
accumulators would keep its value but gain a percent or two), the
occupancy layout (interleaving the trace and diagonal maps per cell would
save a cache line a diagonal step, but touches every map's indexing), a
radix or bucket queue for the heap (needs monotone keys; the weighted
heuristic, the reach bonus and float rounding do not guarantee them).

## Measured (2026-10-25)

Snapshots x1820 (main 23f629e) and x14381 (5a47f18), side by side on
the same physical cores (one on CPUs 8 and 9 or 10 and 11 or 12 and 13
or 14 and 15, the other on their SMT siblings, 16 higher), two search
threads each, pinned rates (`PCB_WORK_RATE=3.15e6,150e6`),
`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`, then again with the
binaries' CPUs swapped (two pairs a board; the machine's load 18 to 60
from other agents' runs). Cells: the two pairs' CPU seconds of the
router process (x1820's pair first, kicad-cli's checks left out); the
change, the geometric mean of the two pairs' ratios; expansions per CPU
second; the route's expansions.

| board | budget | x1820 CPU s | x14381 CPU s | CPU | x1820 M expansions / CPU s | x14381 | expansions | identical |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Castor | 300 | 479 / 306 | 429 / 273 | -10.7 % | 0.91 / 1.42 | 1.01 / 1.59 | 434 M | yes (2 pairs) |
| jetson | 300 | 620 / 381 | 561 / 346 | -9.4 % | 1.01 / 1.65 | 1.12 / 1.82 | 628 M | yes (2 pairs) |
| k30-SBC | 300 | 651 / 440 | 569 / 398 | -11.1 % | 0.76 / 1.13 | 0.87 / 1.25 | 497 M | yes (2 pairs) |
| Sisu | 300 | 673 / 507 | 609 / 463 | -9.0 % | 0.67 / 0.88 | 0.74 / 0.97 | 448 M | yes (2 pairs) |
| SmartSpin2k | 1200 | 1074 / 891 | 954 / 769 | -12.4 % | 2.02 / 2.43 | 2.27 / 2.82 | 2166 M | yes (2 pairs) |
| protoesp | 1200 | 933 / 761 | 842 / 674 | -10.6 % | 0.87 / 1.07 | 0.96 / 1.20 | 811 M | yes (2 pairs) |

**Identical** in every pair: the routed board's SHA-256, every
fingerprint line, the expansions and searches, and the verdict apart from
its work and wall seconds (`next_actions` with the output directory
replaced). x14381's fingerprints on the four fingerprint boards are also
those of the nightly of 2026-10-25 (356c8db, four threads), at two and at
four threads; at four threads (CPUs 0-3 against their siblings 16-19, one
pair) Castor took 311 against 279 CPU seconds (-10.3 %), k30-SBC 427
against 383 (-10.3 %). The tests (`scripts/check.sh --fast`) pass.

**The changes one at a time** (indicative: one pair each, k30-SBC at 60 s
of work on sibling CPUs unless said otherwise): the hop test by
coordinates (the first version, x14380's) -5 to -6 % on Castor at 30 s
(separate cores, two pairs); `since_via` on top -4.3 % and -4.4 % (both
orientations); the heap's empty root -7 % on Castor at 30 s (separate
cores; the heap's samples in a k30-SBC profile at 60 s fell from 16.8 k
to about 9.5 k of the router's 110 k); the neighbourhood prefetch -6.4 %
(cycles -8.5 %) against the same binary without it; its prefetches
without bounds checks -2.7 % (instructions -4.3 %). The k30-SBC pairs ran
at load 50 to 60; on Castor at 30 s at load 16 to 20 (separate cores)
the neighbourhood prefetch gained 2 to 3 %.

## The work clock

Every budget is work: search expansions at `expansions_per_second`.
Where the rates are pinned (`PCB_WORK_RATE`, as every tier and run.py
pins them at the reference's 3.15e6), a faster binary does the same
expansions and gives the same board in less CPU: the changes above are
invisible to the results. Where they are measured (the default of a
command that routes or places: `calibrate` in
crates/pcb-kicad/src/work_rate.rs), the rate is the reference's scaled by
a canned search's speed on this machine against its speed on the
reference (`REFERENCE_CANNED_EXPANSIONS_PER_SECOND`, 6.9e6, measured with
x1820's search). The canned search is the router's search, so the new
binary measures a higher rate and a second of budget buys more
expansions: on this machine the canned search ran 1.12 to 1.16 times as fast with
x14381 as with x1820 (the median of twelve calibrations, four pairs on
SMT siblings, load 20 to 50), so an unpinned run of x14381 applies a rate
about 15 % higher and spends that many more expansions in the same budget,
and its results differ from x1820's (as with any change of the measured
rate). The semantics stay as documented ("a second
of work is about a second of this machine's wall clock"); recalibrating
`REFERENCE_CANNED_EXPANSIONS_PER_SECOND` (which would keep the measured
rate where it was and hand the saving back as wall time instead of more
search) is left to a separate package.

## The second router round (2026-10-27)

The search again, after the first round and the memory round (whose
four-bit occupancy and stamp lists changed what a step and a restamp
read): branch router-profile-2, snapshots x14960 to x14967 (x14967 the
three changes on main 71953b0), against main's x1849 (a4e5de2) and
x1851 (71953b0; nothing between them changed the routing).

### How

As above ("How to profile"): the profiling binary of main's a4e5de2 (and
of the branch with the first two changes, 24fb127f, for the profile
after), one route per board under `perf
record -F 499 -m 16 -g`, two search threads on two CPUs, pinned rates,
`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`; the four
fingerprint boards at a quarter budget (300 s of work, v16 placements),
SmartSpin2k and protoesp at the task budget (1200 s, v19), the machine's
load 26 to 37 from other agents and the nightly.

`--sort srcline` names only a sample's innermost inlined frame, so the
first round's "inlined std" (indexing, pointer arithmetic: 12 to 14 %)
and part of its "heap" were lines of std whose caller it did not show.
This round also attributes each sample of the search to its outermost
line of search.rs through the inline chain: `perf script -F ip,sym,symoff
--no-inline -G` for the addresses (seconds), `addr2line -a -i` on the
profiled binary for each address's chain; a sample counts for the heap
when its innermost frame is heap.rs or its search line is a push or a pop,
for the occupancy when it is occupancy.rs or stamping.rs
(`occupancy_seen`), for the static maps when it is grid.rs, else by its
search line's range. The scripts are not committed (scratch); the method
is these two commands.

Two counts came from a throwaway build (counters in the search, not
committed), k30-SBC at 60 s and protoesp at 200 s of work:

| | k30-SBC | protoesp |
| --- | ---: | ---: |
| pops per expansion | 1.63 | 1.70 |
| of the pops, of a node already closed (stale) | 39 % | 41 % |
| pushes per expansion | 2.43 | 1.82 |
| pops whose key is below the previous pop's | 0.20 % | 0.41 % |
| the open list's largest size | 245 k | 135 k |
| expansions guided by the coarse map / at most 4 points / 5 to 12 | 12 / 85 / 4 % | 34 / 62 / 4 % |
| step lookups of the occupancy that find no net (trace map, diagonal map, via map) | 89, 93, 68 % | |
| SmartSpin2k (200 s): the same | 68, 80, 40 % | |
| of those lookups, the 16 cells of the word all free (k30-SBC; SmartSpin2k) | 79, 81, 51 %; 47, 52, 23 % | |

### Where the route's CPU goes now (main a4e5de2)

**The top symbols by self time** (% of the route's samples; in brackets
where not among the board's top 15):

| symbol (self) | Castor | k30-SBC | jetson | Sisu | SmartSpin2k | protoesp |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `Router::search` | 70.23 | 76.05 | 78.77 | 74.32 | 88.79 | 87.68 |
| `Router::search::{closure#2}` (the heuristic) | 7.16 | 7.75 | 6.24 | 5.03 | 5.08 | 5.61 |
| `Router::stamp` | 7.53 | 5.26 | 3.24 | 9.41 | 2.59 | 3.31 |
| `Router::plan_corridor` | 1.05 | 2.26 | 2.24 | 1.61 | 0.56 | 0.34 |
| `Router::plan_corridor::{closure#2}` | 1.00 | 2.24 | 2.12 | 1.49 | 0.54 | 0.30 |
| `Router::plan_corridor::{closure#3}` | 0.67 | 1.51 | 1.54 | 1.09 | 0.32 | 0.18 |
| `PourMap::build` | 2.97 | 0.74 | (0.09) | 0.55 | - | 0.12 |
| `StateMap insert (hashbrown)` | 0.31 | 0.30 | 0.92 | 0.60 | 0.11 | 0.14 |
| `Shape::distance_to_point` | 1.14 | 0.25 | 0.16 | 0.53 | 0.03 | 0.11 |
| `Router::analyze_pours_uncounted` (an iterator closure) | 1.07 | 0.24 | (0.03) | 0.17 | - | 0.05 |
| `geometry::point_in_polygon` | 0.46 | 0.12 | 0.20 | 0.49 | (0.02) | 0.18 |
| `Router::mask_own` | (0.18) | 0.10 | 0.11 | 0.51 | 0.17 | 0.21 |
| `geometry::segment_segment_distance` | (0.04) | (0.01) | 0.14 | 0.60 | (0.02) | (0.02) |
| `Router::branch_conflicts` | (0.03) | (0.06) | (0.09) | (0.07) | 0.42 | 0.06 |
| `fill::Index init (OnceCell)` | 0.37 | 0.22 | (0.03) | (0.06) | - | (0.04) |
| `Router::unstamp` | (0.10) | (0.07) | (0.06) | (0.10) | 0.09 | 0.28 |
| `Router::kicad_spokes` | 0.49 | (0.08) | (0.01) | (0.01) | - | (0.02) |
| `FillProbe::blocker` | 0.49 | (0.04) | (0.00) | (0.00) | - | (0.02) |
| `Router::route_net_with_arrays` | (0.07) | 0.09 | 0.14 | (0.12) | (0.03) | 0.04 |
| `PourMap::pieces_near` | - | (0.00) | - | 0.43 | - | (0.02) |
| `verify::verify_with_rules` | (0.01) | (0.00) | 0.32 | (0.01) | (0.01) | (0.00) |
| `HashSet<usize> insert (SipHash)` | 0.24 | (0.07) | (0.01) | (0.01) | - | (0.01) |
| `Shape::aabb` | (0.08) | (0.01) | (0.03) | (0.06) | 0.14 | (0.01) |
| `PourMap::piece_near` | - | - | - | 0.26 | - | (0.02) |
| `Router::node_class` | (0.06) | (0.03) | (0.02) | (0.03) | 0.06 | (0.02) |
| `kicad_sexpr::parse`, `Parser::atom` | (0.02) | (0.02) | 0.25 | (0.01) | (0.02) | (0.02) |
| `Router::stitching_vias` | (0.03) | 0.13 | - | - | - | (0.01) |
| `Router::route_net_seq`, `Router::stub_is_clear` | (0.01) | (0.02) | (0.04) | (0.10) | 0.09 | (0.04) |

The search is 70 to 89 % of every route, with its heuristic 77 to 94 %.
`Router::stamp` rose from 1.4-5.2 % to 2.6-9.4 % (the memory round's
stamp lists as varints, decoded in the restamp's two passes).

**The hot spots by the first round's method** (innermost line, lines
below 0.3 % of the samples lumped, the lines grouped by our reading of
its categories; the first round's x1820 in brackets):

| hot spot | Castor | k30-SBC | jetson | Sisu | SmartSpin2k | protoesp |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| inlined std (indexing, iterators) | 12.9 (9.6) | 12.4 (9.7) | 12.6 (9.1) | 12.9 (10.0) | 13.4 (9.0) | 14.4 (10.6) |
| occupancy lookups | 10.9 (6.7) | 12.2 (8.1) | 12.7 (8.6) | 11.4 (8.1) | 16.8 (11.4) | 14.4 (10.4) |
| search, lines below 0.3 % | 10.1 (12.0) | 10.2 (10.4) | 10.1 (9.9) | 10.2 (10.3) | 10.6 (10.1) | 11.0 (10.4) |
| neighbour steps (node, clearance, cost) | 9.4 (10.8) | 9.9 (12.8) | 10.6 (13.9) | 10.9 (12.9) | 11.3 (13.1) | 12.9 (14.8) |
| heap (push, pop) | 8.6 (14.8) | 9.8 (16.2) | 10.3 (17.4) | 9.0 (16.0) | 11.9 (18.2) | 11.9 (17.9) |
| heuristic (search closure 2) | 7.2 (5.7) | 7.8 (6.6) | 6.2 (5.1) | 5.0 (4.7) | 5.1 (3.9) | 5.6 (4.6) |
| stamping (Router::stamp) | 7.5 (4.4) | 5.3 (3.1) | 3.2 (2.2) | 9.4 (5.2) | 2.6 (1.4) | 3.3 (1.8) |
| unattributed (search.rs:0, spills) | 5.3 (7.5) | 5.3 (6.6) | 6.1 (8.7) | 5.8 (8.0) | 7.0 (8.7) | 6.9 (8.4) |
| static maps (edges, vias, layers) | 4.6 (3.0) | 4.7 (2.8) | 5.0 (3.2) | 4.3 (2.9) | 6.8 (4.4) | 5.0 (3.4) |
| prefetches (the first round's) | 3.4 (-) | 3.7 (-) | 5.0 (-) | 3.5 (-) | 3.3 (-) | 3.3 (-) |
| via steps | 2.6 (3.9) | 4.5 (6.2) | 3.3 (5.3) | 3.7 (4.8) | 2.6 (3.3) | 3.3 (3.0) |
| pop and expansion set-up | 2.4 (1.3) | 3.3 (1.5) | 3.0 (1.9) | 2.6 (1.7) | 4.5 (2.5) | 4.3 (2.4) |
| tile planner (plan_corridor) | 2.7 (2.5) | 6.0 (5.6) | 5.9 (5.8) | 4.2 (4.8) | 1.4 (1.2) | 0.8 (0.8) |
| pours (maps, analyses, spokes) | 7.5 (6.5) | 1.8 (1.4) | 0.6 (0.5) | 2.7 (3.3) | 0.1 (0.0) | 0.6 (0.3) |
| hop test (hops_from_a_via) | 0.0 (6.5) | 0.0 (5.8) | 0.0 (3.8) | 0.0 (3.3) | 0.6 (8.3) | 0.4 (8.9) |
| hashing (hash maps) | 1.0 (0.5) | 0.5 (0.2) | 1.0 (0.7) | 0.8 (0.6) | 0.1 (0.1) | 0.2 (0.1) |

The first round's changes show: the hop test is gone (since_via), the
heap's own lines fell from 15-18 % to 9-12 % (the empty root); the
occupancy rose from 7-11 % to 11-17 % (the four-bit counts are a shift
and a mask more, and the other parts got faster); the rest moved by
their shares.

**The hot spots through the inline chain** (each sample at its outermost
search line; the cells: main a4e5de2 / the branch with the first two
changes below, 24fb127f on a4e5de2; shares of each route's own samples, so a
part that did not change grows as the route shrinks):

| hot spot | Castor | k30-SBC | jetson | Sisu | SmartSpin2k | protoesp |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| neighbour steps (node, clearance, cost) | 17.4 / 20.6 | 17.2 / 20.5 | 17.4 / 19.0 | 18.9 / 21.0 | 18.3 / 20.7 | 22.0 / 24.2 |
| heap (push, pop, `next_hint`) | 15.9 / 17.1 | 17.8 / 19.0 | 18.7 / 19.3 | 16.1 / 16.5 | 20.8 / 22.7 | 20.3 / 21.2 |
| occupancy lookups | 11.3 / 5.6 | 12.7 / 6.4 | 13.3 / 6.0 | 12.1 / 5.7 | 17.7 / 12.3 | 15.6 / 10.3 |
| heuristic (search closure 2) | 7.2 / 7.8 | 7.8 / 8.7 | 6.2 / 7.1 | 5.0 / 5.8 | 5.1 / 6.4 | 5.6 / 6.5 |
| via steps | 5.7 / 6.1 | 7.5 / 8.1 | 6.1 / 6.4 | 6.3 / 6.0 | 4.6 / 3.4 | 5.7 / 5.3 |
| stamping (Router::stamp) | 7.5 / 7.7 | 5.3 / 5.4 | 3.2 / 3.2 | 9.4 / 10.0 | 2.6 / 2.8 | 3.3 / 3.5 |
| pop and expansion set-up | 5.2 / 4.1 | 5.8 / 4.1 | 6.1 / 4.1 | 6.3 / 4.6 | 8.0 / 5.4 | 7.2 / 4.8 |
| unattributed (search.rs:?, spills) | 5.3 / 4.8 | 5.3 / 4.9 | 6.1 / 5.8 | 5.7 / 5.4 | 6.9 / 6.0 | 6.9 / 6.5 |
| static maps (edges, vias, layers) | 4.8 / 3.9 | 4.9 / 4.1 | 5.2 / 5.3 | 4.5 / 4.3 | 7.2 / 6.2 | 5.3 / 5.0 |
| prefetches | 3.3 / 5.3 | 3.8 / 6.2 | 5.0 / 9.6 | 3.5 / 6.8 | 3.2 / 8.1 | 3.1 / 7.5 |
| tile planner (plan_corridor) | 2.7 / 2.9 | 6.0 / 6.5 | 5.9 / 6.6 | 4.2 / 4.5 | 1.4 / 1.4 | 0.8 / 0.8 |
| pours (maps, analyses, spokes) | 7.5 / 8.0 | 1.8 / 2.0 | 0.6 / 0.7 | 2.7 / 3.4 | 0.1 / 0.1 | 0.6 / 0.8 |
| hashing (hash maps) | 1.0 / 1.0 | 0.5 / 0.5 | 1.0 / 1.0 | 0.8 / 0.8 | 0.1 / 0.1 | 0.2 / 0.2 |
| hop test (hops_from_a_via) | 0.3 / 0.4 | 0.3 / 0.3 | 0.2 / 0.2 | 0.3 / 0.3 | 1.1 / 1.3 | 0.9 / 1.0 |
| search set-up and path | 0.3 / 0.3 | 0.4 / 0.3 | 0.5 / 0.5 | 0.5 / 0.4 | 0.8 / 0.9 | 0.4 / 0.4 |

**The costliest lines** (same cells; main's search.rs line in brackets):

| line | Castor | k30-SBC | jetson | Sisu | SmartSpin2k | protoesp |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| a step's push (456) | 8.2 / 8.8 | 8.4 / 8.9 | 9.2 / 9.5 | 8.0 / 8.2 | 9.7 / 10.5 | 9.4 / 9.8 |
| the pop (232) | 6.6 / 7.0 | 7.8 / 8.2 | 8.2 / 8.1 | 7.1 / 7.1 | 10.2 / 11.0 | 9.8 / 10.2 |
| a diagonal step's clearance count (402) | 6.3 / 1.7 | 7.8 / 2.2 | 8.8 / 2.1 | 7.8 / 2.1 | 9.7 / 4.3 | 8.1 / 3.6 |
| a step's count (391) | 2.9 / 3.0 | 3.3 / 4.2 | 3.5 / 3.5 | 3.1 / 3.2 | 5.4 / 6.6 | 5.5 / 5.9 |
| a step's neighbour closed (369) | 3.0 / 3.0 | 3.4 / 3.3 | 3.5 / 3.6 | 3.0 / 3.1 | 3.9 / 4.1 | 4.1 / 4.4 |
| the node's blocked steps, `edges_at` (272) | 2.7 / 3.4 | 3.3 / 4.0 | 3.8 / 4.4 | 3.1 / 3.5 | 4.3 / 5.7 | 2.9 / 3.9 |
| the popped node closed (234) | 2.0 / 0.3 | 2.6 / 0.4 | 2.6 / 0.4 | 2.3 / 0.3 | 4.0 / 0.6 | 3.5 / 0.6 |
| the via's count (485) | 2.6 / 1.2 | 2.6 / 1.5 | 2.2 / 1.3 | 2.2 / 1.3 | 4.1 / 3.1 | 3.7 / 2.7 |
| `via_allowed` (487) | 1.4 / 1.4 | 1.2 / 1.0 | 1.7 / 2.0 | 1.6 / 1.9 | 2.4 / 1.9 | 2.2 / 2.2 |
| the via's `free_layers` (537) | 1.5 / 0.5 | 1.2 / 0.3 | 0.7 / 0.2 | 0.8 / 0.2 | 1.6 / 0.4 | 1.1 / 0.2 |

The search waits on memory: the costliest lines are loads of maps a
row or a map away from the node (the diagonal steps' clearance maps were
not among the first round's prefetches), the heap's levels and the
popped node itself, whose record is known only when it is popped.

### Candidates checked

Each measured as one change against the binary without it, k30-SBC at
60 s of work, two pairs on SMT siblings in both orientations (the
router's CPU seconds; cells: the two orientations' change):

- **A radix heap** (`RadixHeap`, x14960; branch
  router-profile-2-radix-trial, 8190db4b): the keys are not small
  integers (the estimate's float bits above the state), but they are
  almost monotone (0.2 to 0.4 % of the pops below the previous pop), so
  a radix heap over the 64-bit keys with a 4-ary heap for the keys below
  the last pop pops the same keys (the heap tests pass for it). **+4.9 %
  and +5.0 %** (instructions +3.8 %): its pushes are appends, but a key
  moves through several buckets on its way down, and the 4-ary heap's
  push into the empty root (the first round) already stops near the
  root. Not taken. A lazier decrease-key would remove the 39-41 % stale
  pops but needs a heap position per state (the 12-byte node has no
  room). Not tried.
- **The heuristic inlined** (a struct whose `at` inlines into every
  step, the same arithmetic; x14963, branch
  router-profile-2-heuristic-trial, 29113532): **+1.9 % and +0.9 %**,
  instructions the same. The closure's call is cheap beside its loads;
  inlining nine copies costs registers (the search spills already). Its
  loop over up to 12 points is vectorised by LLVM, and 85 % of the
  expansions (k30-SBC) have at most 4 points. Not taken.
- **A summary bit per word of the occupancy** (16 cells, set while any
  counts a net; a clear bit answers a lookup from a map a 64th the size;
  x14965, branch router-profile-2-summary-trial, e74df3ed): **-3.3 % and
  -1.0 %** on k30-SBC, **-0.9 % and +3.8 %** on SmartSpin2k at 200 s
  (fewer free words: 47 % against 79 %), instructions +2 to 4 %. Not
  taken: a gain only on the sparse boards, and the prefetches below took
  most of the same loads.
- **The steps' bounds checks and branches**: the window test is per
  expansion already (`interior`), the edge, corridor and closed tests
  are branches on loaded values; the samples sit on the loads, not the
  tests. Not changed.
- **The via steps' layers**: `free_layers` decides most layers in one
  word; their cost was the loads of the cell's maps, which change 1
  prefetches.

### The changes (bit-identical)

Three prefetches (hints only: they change no value, so every route is
the same by construction):

1. **The rest of the expansion's maps** (search.rs, after the first
   round's prefetches): the diagonal steps' clearance counts (both
   orientations at the node's cell, where the steps towards +x start, and
   at the cells a row below and above, where the steps towards -x start),
   and what the via step reads of the cell: its via count, `via_blocked`,
   `free_layers` (`NodeBits::prefetch`, `LayerBits::prefetch`),
   `own_via_near` and the via history. Line 402 fell from 6-10 % to
   2-4 %, the via's count by a quarter to a half, `free_layers` by two
   thirds or more.
   Alone (x1849 against x14962): **-3.1 % and -0.8 %**; on the radix heap
   (x14960 against x14961) -2.2 % and -2.8 %.
2. **The next pop's search node, an expansion ahead**
   (`KeyHeap::next_hint`): after a pop the root is empty (the first
   round) and the least of its four children is the key the next pop
   takes unless the expansion pushes a lesser one; its search node is
   prefetched before the expansion starts. The popped node's closed test
   (line 234) fell from 2-4 % to 0.3-0.6 %. (x14963 against x14964, both
   with the heuristic trial above): **-3.2 % and -4.5 %**.
3. **The next pop's blocked-step bits** (`StaticMaps::prefetch_edges`,
   with change 2: the node's layer from its state, one division off the
   path): `edges_at`, the expansion's first map read (3 to 6 % after
   change 2). (x14964 against x14966): **-2.5 % and -2.6 %**; SmartSpin2k
   at 200 s -2.7 % and -2.0 %.

Each prefetch is an instruction: instructions rise, cycles fall.

### Measured (2026-10-27)

Snapshots x1851 (main 71953b0) and x14967 (c181d7d, the three changes
rebased on it), side by side on SMT siblings (one binary on two of the CPUs 0 to 15,
the other on their siblings 16 higher; the second pair with the
binaries' CPUs swapped, run at the same time on the next two cores), two search threads, pinned rates,
`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`, the machine's
load 30 to 38 (other agents, the nightly from 02:00). Cells: the two
pairs' CPU seconds of the router process (x1851's first orientation
first); the change, the geometric mean of the two pairs' ratios;
million expansions per CPU second; the route's expansions.

| board | budget | x1851 CPU s | x14967 CPU s | CPU | cycles | x1851 M expansions / CPU s | x14967 | expansions | searches | identical |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Castor | 300 | 465 / 465 | 429 / 452 | -5.4 % | -5.4 % | 0.93 / 0.93 | 1.01 / 0.96 | 434 M | 8267 | yes (2 pairs) |
| jetson | 300 | 578 / 579 | 564 / 492 | -9.0 % | -9.5 % | 1.05 / 1.04 | 1.07 / 1.23 | 604 M | 10416 | yes (2 pairs) |
| k30-SBC | 300 | 616 / 612 | 590 / 583 | -4.4 % | -4.7 % | 0.81 / 0.81 | 0.84 / 0.85 | 497 M | 8758 | yes (2 pairs) |
| Sisu | 300 | 751 / 752 | 658 / 687 | -10.5 % | -11.5 % | 0.82 / 0.81 | 0.93 / 0.89 | 613 M | 10012 | yes (2 pairs) |
| SmartSpin2k | 1200 | 1342 / 1331 | 1223 / 1290 | -6.0 % | -5.7 % | 1.61 / 1.63 | 1.77 / 1.68 | 2166 M | 32073 | yes (2 pairs) |
| protoesp | 1200 | 1327 / 1302 | 1331 / 1299 | +0.0 % | -0.3 % | 0.61 / 0.62 | 0.61 / 0.62 | 810 M | 11491 | yes (2 pairs) |

The CPU falls 4 to 11 % on five boards; protoesp did not change (+0.3
and -0.3 % in the two orientations). Instructions rose 3.6 to 5.6 % (the
prefetches). x1851's two runs of a board agree within 2 %, x14967's
differ by up to 13 % (jetson 564 and 492 s): the prefetches' gain depends
on what the sibling CPU and the other cores load at the time.

**Identical** in every pair: the routed board's SHA-256, every
fingerprint line, the expansions and searches, and verdict.json apart
from its work, wall, rate, memory and path fields. Castor's board is the
first round's (SHA-256 prefix dad3a6cb3787b575, 434 M expansions), as
are k30-SBC's expansions (497 M). The expansion tier (46 boards, 8 x 1,
30 s): 46 of 46 routed boards byte-identical to x1850's (main ccb9504f,
which routes as x1851), the rows identical apart from their timing
fields and the intake's speed class (`outcome`, from the route's
seconds: five clean_slow to clean_medium, one incomplete_at_budget to
incomplete_early), 557 unconnected, 0 new errors, 4263 vias both. The tests (`scripts/check.sh --fast`, clippy with
warnings as errors) pass.

The pairs vary more than the first round's (jetson's two orientations,
run at once on two core pairs: -2.4 and -15.1 %).

**The work clock.** As after the first round, pinned rates (every tier,
run.py) do the same expansions in less CPU. The canned search that
measures an unpinned rate (crates/pcb-kicad/src/work_rate.rs) fits the
caches, where a prefetch saves nothing: six calibrations side by side on
one CPU and its sibling (`calibrate-work-rate 1`, alternating) gave
medians of 2.074 M (x1851) and 2.065 M (x14967) expansions a second, the
pairs' ratios 0.96 to 1.02. An unpinned run measures about the rate it
measured before and spends the saving as wall time, not as more search.
(The routes' own start-up calibrations, taken while the pair's other
binary started too, ranged 0.83 to 1.04; single runs under load.)

### Not taken, and what is next

- The heap is now the largest part (16 to 23 % through the inline chain):
  a 4-ary heap of 8-byte keys, 1.6 to 1.7 pops an expansion, 40 % of
  them stale. A position per state for a decrease-key (4 bytes a state
  more: the memory round's budget) or an 8-ary layout on whole cache
  lines are the candidates left.
- The steps' counts (line 391, 3 to 7 %) are prefetched since the first
  round but still wait: the rows' prefetches are issued only an
  expansion's set-up ahead. Prefetching the next pop's rows with its node
  (change 2) would give them an expansion ahead, at 13 more prefetches an
  expansion.
- `Router::stamp` (3 to 10 %), outside the search: the varint stamp
  lists' decoding.

## The layout round (2026-10-25)

Everything a layout does outside the router's search: the placer (global
placement, anneal, legaliser, refinement, the crossings, the escape
demand), the board lowering (the KiCad file's parse, the geometry, the
rules), the pour model, the write-back, and kicad-cli's checks.

### How

`layout-kicad-board` under `perf record -F 499 -m 16 -g` (the profiling
binary as above), the task's source prepared as run.py prepares it (copy,
`strip-kicad-tracks`, `unplace`, constraints.json, layout.json; tasks v7),
at the task budgets (MegaDrive and Glasgow 1500 s, the short tier's
1337-v4.0 and OpenFC a third: 500 s), `PCB_WORK_RATE=3.15e6,150e6
PCB_ROUTER_NO_WALL_GUARD=1 PCB_ROUTER_FINGERPRINT=1 RAYON_NUM_THREADS=2`,
two CPUs each (`taskset`), a DRC cache of its own per run
(`PCB_MAKER_CACHE_DIR`, cold: every check runs kicad-cli) and kicad-cli
through a logging wrapper (`PCB_KICAD_CLI`: each call's arguments, wall
and CPU). `perf record` follows the children, so kicad-cli's samples are
in the profile under its own command name. The layout's own phases come
from its log's timestamps (`layout: placing`, `placed`, `routed`, `done`).

Two traps: `perf script` resolves inlined frames through addr2line and
took minutes per thousand samples (`--no-inline`: seconds for 600 000);
libm has no frame pointers, so a sample in libm names the caller's
caller, not the caller (glibc's own symbols, `objdump -d
/usr/lib/libm.so.6`, at the sample's offset in the mapping that
`--show-mmap-events` gives, name the libm function: `__sincos_fma`,
`__hypot_finite`).

### Where the layout's time goes (x1824's code, 2026-10-25)

**CPU** (% of the layout process's samples; kicad-cli's beside them as a
share of all):

| phase | MegaDrive | Glasgow | 1337-v4.0 | OpenFC |
| --- | ---: | ---: | ---: | ---: |
| placer (place_kicad_board and the placement race's lowering) | 30.9 | 60.9 | 30.0 | 32.6 |
| of it: legaliser and refinement | 16.7 | 30.0 | 7.9 | 6.6 |
| of it: anneal | 8.6 | 12.2 | 14.9 | 19.2 |
| of it: escape demand | 3.6 | 12.3 | 3.2 | 3.8 |
| of it: ratsnest crossings | 2.0 | 6.2 | 0.3 | 0.3 |
| of it: global placement | 0.0 | 0.2 | 3.5 | 2.6 |
| router | 68.8 | 39.0 | 68.6 | 66.8 |
| of it: the search (with its heuristic) | 58.7 | 35.5 | 51.5 | 54.6 |
| of it: pour analyses, maps and spokes | 2.1 | 0.3 | 12.7 | 6.1 |
| of it: stamping | 4.6 | 1.1 | 2.5 | 3.5 |
| of it: tile planner | 2.4 | 1.5 | 0.5 | 0.8 |
| lowering, the file's parse, encode, write-back | 0.00 | 0.00 | 0.02 | 0.01 |
| calibration, verification, silkscreen | 0.3 | 0.1 | 1.3 | 0.5 |
| kicad-cli (share of all samples) | 1.0 | 0.3 | 3.8 | 2.1 |

The lowering of the board, the KiCad file's parse and the board's
encoding for kicad-cli are below a tenth of a percent on every board (37
lowerings of 1337-v4.0 took 0.2 s in all): not worth a change. The placer
is a third of the CPU on three boards and three fifths on Glasgow (its
placement uses its whole 600 s of work).

**Wall** (seconds, x1824 in the measurement below, the two runs of each
board; the placement's wall includes the memory ledger's waits for the
placement race's routers, up to 715 s on MegaDrive, and so do the
totals):

| phase | MegaDrive | Glasgow | 1337-v4.0 | OpenFC |
| --- | ---: | ---: | ---: | ---: |
| start (rates, check model) | 3 / 2 | 2 / 3 | 2 / 3 | 2 / 2 |
| placement (with the ledger's waits) | 590 / 1222 | 1148 / 1470 | 51 / 41 | 266 / 102 |
| race, first route, moves, polish, the kept board's check | 1240 / 1321 | 288 / 299 | 184 / 124 | 77 / 60 |
| final ladder or narrow-signal attempts, their checks | 4 / 5 | 465 / 465 | 4 / 4 | 226 / 205 |
| kicad-cli in all (DRC, previews, version) | 14.7 / 16.4 | 7.8 / 6.5 | 6.6 / 6.1 | 14.8 / 13.2 |
| total | 1837 / 2550 | 1904 / 2237 | 242 / 171 | 572 / 368 |

The write-back after the layout's last line (silkscreen labels, the
reports) is 0.1 s.

kicad-cli runs once per DRC (`pcb drc --refill-zones`) and once per
preview (`pcb export svg`, after every DRC), plus `version` once: 1 to 5
DRCs per layout (MegaDrive 1, 1337-v4.0 2, Glasgow 3, OpenFC 5), 1.2 to
22 s each uncached (MegaDrive's one 14 to 22 s), 0.4 to 0.8 s per
preview; 6 to 16 s of a layout's 171 to 2550 s of wall (0.3 to 3.6 %). No
DRC repeats a board within a run (the
cache, cold here, answered none: each check is a board the ladder or the
starved-thermal repair changed), so none can be dropped or merged without
changing what is checked. The previews of ladder attempts that are not
kept (OpenFC: 4 of 5) are 0.5 s each; rendering only the kept one would
save 1 to 2 s a layout and was not taken.

**The top symbols outside the search** (self time, % of the layout
process's samples; in brackets where not among the board's top 15):

| symbol (self) | MegaDrive | Glasgow | 1337-v4.0 | OpenFC |
| --- | ---: | ---: | ---: | ---: |
| libm (all functions) | 2.49 | 10.24 | 7.61 | 2.80 |
| `placer buckets::Grid::query` | 7.35 | 3.84 | 2.45 | 3.01 |
| `placer anneal::State::overlap_with` | 1.07 | (1.04) | 2.05 | 6.62 |
| `placer legal::refine_with` | (0.19) | 5.27 | (0.09) | (0.09) |
| `placer legal::refine_with::{closure#2}` | (0.50) | 4.95 | (0.80) | (0.58) |
| `router router::Router::stamp` | 4.27 | (0.97) | 2.34 | 3.22 |
| `router pour::PourMap::build` | (0.37) | (0.05) | 4.05 | 1.67 |
| `placer crossings::CrossingState::count` | 1.16 | 3.76 | (0.13) | (0.16) |
| `placer escape::channel` | (0.10) | 3.56 | (0.36) | (0.75) |
| `placer constraints::Constraints::bare` | 1.32 | 3.49 | (0.64) | 0.99 |
| `placer global::global_place` | (0.02) | (0.10) | 3.44 | 2.15 |
| `round` | 1.45 | 3.18 | 1.48 | 1.27 |
| `placer legal::turned_rects (box iterator)` | 2.76 | 1.28 | 1.03 | (0.76) |
| `placer escape::exits` | 2.39 | 2.70 | 0.82 | 1.28 |
| `placer anneal::State::wirelength` | (0.76) | 2.52 | 2.24 | 2.22 |
| `placer constraints::relation_violation` | (0.04) | 2.39 | (0.07) | (0.05) |
| `router geometry::Shape::distance_to_point` | (0.25) | (0.07) | 2.38 | (0.60) |
| `placer legal::overlaps` | 2.35 | (0.79) | (0.64) | (0.36) |
| `placer problem::Component::boxes (box iterator)` | 1.11 | (0.33) | 1.91 | 1.78 |
| `placer constraints::anchor_point` | (0.02) | 1.75 | (0.04) | (0.02) |
| `[unknown]` | 1.00 | (0.75) | 1.54 | 0.95 |
| `router pour::PourMap::pieces_near` | (0.07) | (0.05) | - | 1.54 |
| `placer anneal::anneal` | (0.20) | 1.36 | (0.35) | (0.49) |
| `placer legal::clear_of::core::iter::adapters::copied::Copiedcore::slice::iter::I` | (0.50) | 1.31 | (0.18) | (0.38) |
| `router geometry::point_in_polygon` | (0.20) | (0.02) | 1.19 | (0.30) |
| `router pour::PourMap::piece_near` | (0.05) | (0.03) | - | 1.01 |
| `router router::Router (iterator)` | (0.14) | (0.01) | 0.95 | (0.48) |
| `placer legal::inside_outline` | (0.10) | (0.27) | (0.05) | 0.95 |
| `router router::Router::plan_corridor` | 0.92 | (0.65) | (0.19) | (0.33) |
| `router router::Router::plan_corridor::{closure#2}` | 0.87 | (0.44) | (0.16) | (0.26) |
| `placer legal::far_side_overlap::{closure#0}` | 0.81 | (0.29) | (0.14) | (0.14) |

libm by function: MegaDrive `__sincos_fma` 2.24, `__hypot_finite` 0.19;
Glasgow `__hypot_finite` 6.36, `__sincos_fma` 3.52; 1337-v4.0
`__sincos_fma` 7.90; OpenFC `__sincos_fma` 2.72 (exp and log2 below 0.1).
`round` (1.3 to 3.2 %) and `fmod` (0.2 to 0.6 %) are the same turns'
quarter-turn tests (`rotate`, `turn_of`).

### The changes (bit-identical)

Three hot spots in the placer, all with the same placement by
construction (the placer's work is counted in units of its operations,
not in time, so a faster placer does the same work and stops where it
stopped):

1. **The turns without libm** (problem.rs `sin_cos_degrees`, `rotation`;
   legal.rs `direction_of`). Every box, half extent and turned rectangle
   computed `(-angle).to_radians().sin_cos()` (a call into libm's sincos)
   per box, and `rotate` divided and rounded (`round`, a libm call on
   x86-64's baseline) to find a quarter turn. `sin_cos_degrees` returns
   the bits of that expression for any angle: for the quarter and eighth
   turns of either sign (and both zeros) from a table the same expression
   filled once at run time (`black_box`, so that no compile-time folding
   by another libm enters); `rotation` answers the exact quarter turns
   without dividing; `direction_of` answers them without `rem_euclid`'s
   fmod; `boxes` and `turned_rects` turn once a pose, not once a box.
   Tests compare the bits with the old expressions over 2900 angles.
2. **A near-any relation measures its anchors only as far as needed**
   (constraints.rs `nearest_violation`). Each decoupling capacitor is
   near any of its supply pins (Glasgow: 23 anchors), and its violation
   was the least of `(box_gap - max).max(0)` over all anchors, each with
   a `hypot` (1.2 billion calls in 60 s of Glasgow's placement work, 7 %
   with a zero argument). The least is now taken with two shortcuts that
   keep the value `relation_violation` makes of it: once an anchor lies
   within `RELATION_TOLERANCE` the violation is none whatever the others
   give (the rest are not looked at), and an anchor whose gap along one
   axis already reaches the least so far is skipped without its `hypot`
   (a hypot is at least the larger of its arguments; a test checks that
   on a million pairs, another the violation against the old fold on
   100 000 random cases).
3. **The neighbour lookups** (buckets.rs `Grid::query`;
   constraints.rs `linked_partners`). The bucket grid's query (the
   anneal's overlap sum, the legaliser's and the refinement's legality
   tests: 7.4 % of MegaDrive's layout, 29 % of its placement after the
   first change) tested each item of a cell against its stamp and pushed
   it when new; its samples sat on that test and load. It now writes each
   item into the free slot past the list's end and counts it only when
   new: the same items in the same order, without the branch. The linked
   pairs (`near` relations' tied parts, a `BTreeSet` searched for every
   pair of neighbours by `bare`: 1 to 3.5 %) are looked up in each part's
   list of partners, filled with the set by `link_pairs`.

Not taken: the router's pour analyses (12.7 % of 1337-v4.0, 6 % of OpenFC:
`analyze_pours_uncounted` builds full-lattice masks per analysis; a change
there is a change of the router's pour model, for its own round), the
escape demand's `exits` and `channel` (2.4 to 6 % on the dense boards),
the crossings' counts (`CrossingState::count`, 3.8 % of Glasgow), and the
previews above.

### Measured (2026-10-25)

Snapshots x1824 (main 09299b6) and x14490 (d912ce0), side by side on
sibling CPUs (x1824 on 8,9 / 10,11 / 12,13 / 14,15, x14490 on the SMT
siblings 16 higher; then swapped), two threads, pinned rates,
`PCB_ROUTER_NO_WALL_GUARD=1 PCB_ROUTER_FINGERPRINT=1`, a cold DRC cache
each, the four boards at once beside other agents' runs (load 14 to 18).
CPU seconds are the layout process's own (utime + stime from /proc,
polled; kicad-cli's left out); wall includes the ledger's waits.

Cells: the two pairs (x1824's first orientation first); the change, the
geometric mean of the two pairs' ratios; the longest ledger wait of each
run.

| board | budget | x1824 CPU s | x14490 CPU s | CPU | x1824 wall s | x14490 wall s | wall | ledger waits (old / new, longest) | identical |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| MegaDrive | 1500 | 2338 / 2321 | 2177 / 2183 | -6.4 % | 1837 / 2550 | 1768 / 2502 | -2.8 % | 24, 715 / 62, 743 s | yes (2 pairs) |
| Glasgow | 1500 | 2689 / 2522 | 2287 / 2167 | -14.5 % | 1904 / 2237 | 1657 / 1660 | -19.7 % | 36, 550 / 8, 82 s | yes (2 pairs) |
| 1337-v4.0 | 500 | 275 / 185 | 251 / 166 | -9.6 % | 242 / 171 | 230 / 184 | +1.0 % | 2, 14 / 0, 40 s | yes (2 pairs) |
| OpenFC | 500 | 709 / 598 | 670 / 578 | -4.3 % | 572 / 368 | 575 / 369 | +0.4 % | 134, 0 / 156, 0 s | yes (2 pairs) |

The CPU falls 4.3 to 14.5 % (the placer's share and its geometry decide:
Glasgow's placement is three fifths of its CPU and the near-any relations
are its hot spot). The wall follows where the ledger let the runs alone
(Glasgow's first pair -13.0 %, MegaDrive's -3.8 %; their placements took
867 against 1148 s and 514 against 590 s) and is noise elsewhere: the
ledger's waits for the placement race's routers (up to 743 s) differ
between the two binaries of a pair.

**Identical** in every pair: the placed board's and the routed board's
SHA-256, every fingerprint line (as a set: the ladder's rungs print theirs
in the order they finish), the verdict apart from the version, the
measured rates, the wall seconds and the output directory, and KiCad's
findings by type and count and every violation other than the
unconnected items in full. The unconnected items' named track or zone
differ between runs of kicad-cli on one board: three runs of x1824's
MegaDrive result through `kicad-cli pcb drc` gave three different lists of
the same 301 findings, and OpenFC's two x1824 runs differ from each other
as much as from x14490's. The number of kicad-cli calls is the same in
every pair (no call was removed or merged).

Placement alone (`place-kicad-board`, 100 s of work, one pair each,
indicative): Glasgow 379 to 307 CPU s with the first two changes and 299
with the third, MegaDrive 318 to 293 and 254; 1337-v4.0 and OpenFC with
the first change (200 s of work) 17.0 to 13.3 and 95.5 to 85.7 CPU s;
every placement identical.


## Memory (2026-10-26)

Where a route's peak resident size goes, and three structures shrunk
with the same routes (branch memory-round-3; snapshots x14550 and x14552:
main's code at 280c52a with the profile below; x14553: the changes on
main a905905, measured against main's x1830; x14551: the changes on
280c52a, against x1829). The earlier rounds: router.md, "Memory (2026-10-03)" to
"Memory (2026-10-23)".

### How

heaptrack is not installed; valgrind is, but massif runs a program many
times slower and a route's budget is work, so a quarter-budget route of
these boards would take hours (not tried). Instead the binary profiles
itself: `PCB_ROUTER_MEM_PROFILE=1` (crates/pcb-router/src/mem_profile.rs)
prints `mem-profile:` lines between the run's own log lines:

- `rss high`: a sampler thread reads /proc/self/statm every 10 ms and
  prints each new high of the resident size (every 2 % or 32 MB), with
  the phase, the live heap and the routers alive with their parts;
- the live heap comes from the binary's global allocator
  (`CountingAllocator`: the system's, counting while the profile is on;
  one relaxed load an allocation otherwise);
- each router registers its parts (`Router::memory_parts`: capacities of
  its maps and lists, the static maps once for the routers that share
  them) when it is built, after each negotiation iteration and when it
  finishes, and drops them with it (`mem_profile::Token`); a router's
  parts during an iteration are those of its last report;
- `phase`: parse, parsed, probes, rungs, write-back, check, seeds,
  steps, pour measure (the ladder's stages; a write-back and a check
  run beside other rungs, so the label is the last one set);
- `peak rss` at the end: when the peak came (seconds, phase), the heap
  then and the routers' parts then.

Route mode on the designer's placement (`run.py --tier designer`),
a quarter of the task budget (`--budget-scale 0.25`: 375 s of work), two
search threads, pinned rates, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_NO_WALL_GUARD=1`, one board at a time beside other agents'
runs (the memory ledger, `--min-free-gb 10`).

### Where the peak goes (main's code)

The peak of each board came while several routers were alive: the rung
probes side by side (four routers, each with its own maps, beside the
shared static maps), or a rung and its via reduction's snapshot. The
cells are MB at the peak and the share of the peak resident size
(`rss`; the heap is within a few percent of it).

| | video | katia | 0xCB-1337 panel | SmartSpin2k_Panelized |
| --- | ---: | ---: | ---: | ---: |
| lattice (nodes, layers, classes / searched) | 2498 x 856, 4, 5 / 3 | 2774 x 1077, 4, 10 / 8 | 1081 x 1113, 4, 3 / 1 | 1897 x 2051, 2, 2 / 2 |
| a router's maps (`memory:` line) | 572 MB | 1403 MB | 251 MB | 323 MB |
| peak resident size | 3396 MB | 5933 MB | 1452 MB | 734 MB |
| when | 118 s of 402, probes | 100 s of 321, probes | 73 s of 446, probes | 89 s of 105, a rung's via reduction |
| routers alive | 4 probes | 4 probes | 4 probes | the rung and its snapshot |
| search scratches (own and spares) | 878 (26 %) | 1256 (21 %) | 492 (34 %) | 210 (29 %) |
| occupancy | 530 (16 %) | 1482 (25 %) | 179 (12 %) | 52 (7 %) |
| search records (`hot`) | 522 (15 %) | 1459 (25 %) | 147 (10 %) | 59 (8 %) |
| stamp lists | 843 (25 %) | 523 (9 %) | 69 (5 %) | 81 (11 %) |
| history | 163 (5 %) | 228 (4 %) | 92 (6 %) | 89 (12 %) |
| static maps (shared) | 116 (3 %) | 342 (6 %) | 40 (3 %) | 59 (8 %) |
| stamp marks | 66 (2 %) | 185 (3 %) | 22 (2 %) | 6 (1 %) |
| guard, cover, via cover | 107 (3 %) | 239 (4 %) | 97 (7 %) | 7 (1 %) |
| pour masks (`plane`, `plane_target`) | 24 (1 %) | 68 (1 %) | 110 (8 %) | 0 |
| tiles, nets, branches, footprints, lowered board | 42 (1 %) | 47 (1 %) | 16 (1 %) | 14 (2 %) |
| beside the routers (parse, the ladder's holds, allocator) | 104 (3 %) | 103 (2 %) | 187 (13 %) | 155 (21 %) |

Main's code with the profile: x14550 (fd39b3c; video, katia, the panel)
and x14552 (the same with the via reduction's snapshot reporting its
own parts; SmartSpin2k). The routers' parts are their last reports
(after a negotiation iteration), so a part that grew during the
iteration counts as it was; "beside the routers" is the peak less the
parts. The four probes of a board each hold their own occupancy, search
records, history, guard and cover, stamp lists and scratches; only the
static maps are shared.

The KiCad parse is small beside the routers (10 to 44 MB of heap on
these boards), the lowered boards (`board`: obstacles, nets, pours'
polygons) 2 to 6 MB a router, the corridor instrumentation (atomics) none
to speak of. After the probes, with no router alive, the resident size
stays at 230 to 810 MB while the heap falls to 5 to 400 MB: glibc keeps
the small blocks' arenas; the peak is not there.

### The changes (bit-identical)

1. **The stamp lists as differences** (`Stamped`, router.rs). A net's
   stamps, the (map, cell) pairs its copper covers in order, were a
   32-bit word each (since 2026-10-22). They are read only in order (the
   rip-up, the restamp's two passes, the own-stamp masks, the
   fingerprint), and consecutive stamps lie a row or a column apart on
   one map or on the next of a node's maps: each is now the zigzag
   LEB128 varint of its key's difference from the one before (`map << 32
   | cell`), 1.5 bytes a stamp instead of 4 on Castor and Sisu, and a
   restamp gives back the room the list's doubling left (`trim`). The
   restamp's prefetch reads the list 32 entries ahead with a second
   iterator. The same pairs in the same order by construction (a test
   round-trips the extremes and a random walk).
2. **The search arrays are the process's** (`SearchArrays`,
   `IDLE_ARRAYS`, router.rs; route_net.rs). A search's per-state arrays
   (the search nodes, 12 bytes a state, the via marks and the heap) were
   a router's for its life: its own scratch and a spare per thread of
   its parallel batches, so four probes on two threads held eight sets
   while at most six searches ran at once (four probe threads, two batch
   threads). A net's search now takes idle arrays of its lattice from the
   process (any router's) and gives them back after the net; they go
   with the last router of their lattice (`LatticeUser`). Any search may
   use any arrays of its lattice: it reads only marks of its own
   generation, and the generation travels with the arrays (a batch's
   jobs took any of the batch's scratches before). The per-net masks
   (`own_mark`, the lane) stay with the scratch.
3. **The occupancy in four bits** (`Occupancy`, occupancy.rs). A count
   (how many nets' stamps cover a cell of a class's map) was a byte
   (since 2026-10-23), counts of 255 and more keeping the rest in a side
   map. Two cells now share a byte, counts of 15 and more keep the rest
   in the side map: every count is exact (a test counts two neighbouring
   cells past the saturation and back). No board measured had a count
   above 15. The router's maps (`memory: N MB in maps`, which the ladder
   sizes its routers side by side by) shrink by half the occupancy.

### Measured (2026-10-26)

Main's snapshot x1830 (a905905) against x14553 (the branch rebased on
it), route mode as above, and the
fingerprint boards as the nightly runs them (v16 placements, 1 x 4, a
quarter budget). Peak: `layout_rss_mb`, the route process's own (the
process tree's `peak_rss_mb` adds kicad-cli's checks); maps: the router's
`memory:` line.

| board | x1830 peak MB | x14553 peak MB | saving | maps MB | probes side by side | identical |
| --- | ---: | ---: | ---: | --- | --- | --- |
| katia | blocked | blocked | | 1403 (x14550) | 4 of 4 (x14550) | |
| video | blocked | blocked | | 572 (x14550) | 4 of 4 (x14550) | |
| 0xCB-1337 panel | 1452 | 1120 | -22.9 % | 251 -> 229 | 4 of 4 both | yes |
| SmartSpin2k_Panelized | 745 | 650 | -12.8 % | 323 -> 297 | none (rungs) | yes |
| Castor (fingerprint) | 1073 | 661 | -38.4 % | | 4 of 4 both | yes |
| k30-SBC (fingerprint) | 1630 | 1000 | -38.7 % | | 4 of 4 both | yes |
| jetson (fingerprint) | 1500 | 1017 | -32.2 % | | 4 of 4 both | yes |
| Sisu (fingerprint) | 2114 | 1508 | -28.7 % | | 4 of 4 both | yes |

**Identical** on every board: the routed board's SHA-256, every
fingerprint line (as a multiset), and verdict.json and board-router.json
apart from the work, wall, rate, memory (`maps_mb`, the resident sizes),
DRC-cache and path fields. The same pairs on the branch's first base
(x1829 against x14551, 280c52a) were identical as well, with the same
peaks within 1 to 3 % (the panel 1448 to 1123, SmartSpin2k 746 to 647,
Castor 1084 to 658, k30-SBC 1643 to 981, jetson 1457 to 1018, Sisu 2081
to 1458).

**The composition after** (x14553, the profile on):

| | 0xCB-1337 panel | SmartSpin2k_Panelized |
| --- | ---: | ---: |
| peak resident size | 1120 MB (41 s of 174, probes) | 653 MB (58 s of 85, a rung's via reduction) |
| search arrays (all of the process's) | 297 (were 492 in scratches) | 195 (were 210) |
| occupancy | 89 (were 179) | 26 (were 52) |
| stamp lists | 29 (were 69) | 33 (were 81) |
| search records, history, statics, pour masks | as before | as before |

**katia and video are not measured after.** Their routes were blocked
on 2026-10-26 from 04:40 to past 07:10 by the memory ledger: a stale
harness (the nightly of 2026-10-25, built before the drain's fields)
waited for 24 GB, and while it waits the ledger admits only requests of
4 GB or less (katia 6550 MB expected, video 5720). Their composition
before is in the table above (x14550: main 280c52a with the profile; the
parts the three changes shrink were 3261 MB of katia's 5933 and 2251 MB
of video's 3396). The pairs to run once the waiter is gone (one board at
a time, from the branch's worktree):

```sh
for board in ikajdan__katia__katia Huaqiu-Electronics__ecad-viewer__video; do
  for binary in x1830 x14553; do
    PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1 PCB_ROUTER_MEM_PROFILE=1 \
      python3 benchmarks/agent-tasks/run.py build/mem3-$binary/$board --tier designer --only $board \
      --budget-scale 0.25 --jobs 1 --threads 2 --min-free-gb 10 --binary build/bin/pcb-maker-$binary
  done
done
```

(the profile changes nothing a route computes; x1830 has none and
ignores it).

**CPU** (indicative): k30-SBC at 60 s of work, two threads, pinned rates,
`PCB_SKIP_NATIVE`, x1829 against x14551 on SMT siblings (8,9 / 24,25,
10,11 / 26,27, 12,13 / 28,29), each pair in both orientations: the
router's CPU seconds 131.5 / 138.5, 142.1 / 143.2, 102.3 / 101.7, 83.3 /
92.0, 92.8 / 89.4, 96.1 / 99.1 (the machine's load 18 to 25 from other
agents): +2.5 % as the geometric mean, from -3.7 % to +10.4 %; peak
resident 1109-1127 MB against 795-813 MB (-28 %); identical
fingerprints and boards. The stamping's share beside the search's time
(the iteration lines' `stamp` over `search`) rose from 3.7-4.0 % to
5.2-5.4 %: decoding the stamp lists in the restamp's two passes, the
rip-up and the own-stamp masks. The search's own time did not change
measurably.

### Not taken, and what is next

- **The search records** (`hot`, 10 to 25 % of the peaks): a record
  holds the layer's history beside the class's static map value, so a
  board of S record sets keeps the history S + 1 times. Packing it
  differently changes how the search reads a step (search.rs, another
  package's file at the time).
- **The occupancy of the pour brushes** (classes no net searches with):
  of their maps the pour model reads only the trace and diagonal maps of
  the layers they pour on (video: 2 of 5 classes; about 115 MB of the
  four probes' 530). Their stamps are in the stamp lists the
  fingerprint hashes, and the tiles' claims count them.
- **The pour masks** (`plane`, `plane_target`: a byte a node per pour
  net and layer, 8 % of the panel's peak) as bitsets.
- **Fewer probes side by side than threads allow**: the four probes run
  side by side on two search threads; each probe's own maps are most of
  the peak. Fewer at once would lower the peak further at the price of
  wall time (the outcome is the same by construction; not tried).
- **The idle search arrays** are at most as many as searches ran at once
  (four probe threads and the batch threads): the panel's four probes
  still held five sets (297 MB). A cap at the threads would block a
  probe thread.

