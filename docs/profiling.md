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
after both. The third router round of 2026-10-27 ("The third router
round" below: branch router-profile-3, snapshot x15185 against main's
x1856, edc1d286) gave the open list a decrease-key and the restamp one
decode a list. The second placer round of 2026-10-28 ("The second placer
round" at the end: branch placer-profile-2, measured on main's x1871,
e54c6ada)
profiled the placement again and made it 7 to 44 % cheaper in CPU (22 %
as the geometric mean of fifteen boards) with the same placements. The
third placer round of 2026-10-28 ("The third placer round" at the end:
branch placer-profile-3, measured on main's x1874, 39fc92d6) took the
legaliser's and the refinement's legality tests, queries and crossings:
the placement 4 to 52 % cheaper again (24.5 % as the geometric mean of the
same fifteen boards, 29.4 % of the eleven dense ones). The fourth
router round of 2026-11-04 ("The fourth router round" below: branch
router-profile-4, measured against main's x1889, 951b610a) profiled
what the router gained since the third (the length and neck repairs,
vias in BGA pads, the race's copies) and took the via-in-pad plan's
floods and the pour analysis's window scans and labelling: the same
routes in less CPU. The fourth placer
round of 2026-11-04 ("The fourth placer round" at the end: branch
placer-profile-4, measured against main's x1885, 6a95300a) took the
anneal's overlap sums and crossings, the legaliser's crossings, the
cutouts' outlines and the race's ripped count: the same placements,
the placement 1.4 % cheaper in CPU (geometric mean of the fifteen; video
13.6 %), short of the round's target of 15 %.

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

**Check mode** (`PCB_PLACER_CHECK_OVERLAP=1`, `PCB_ROUTER_CHECK_RIP_UP=1`:
every cache and shortcut against its direct computation at every use) is
observationally identical: a check books no work (it takes back what its
direct computation booked), draws no random number and leaves every
cache as the unchecked path leaves it, so the placement, its booked work
and the routes are those without it, only slower. Until 2026-11-04 the
anneal's overlap check booked its full recount (x1891 in check mode:
eternal-keypad and protoesp placed otherwise, eternal-keypad's placement
139 s of work against 22 s). The placer's
`check_mode_places_as_without_it` tests (crates/pcb-placer/src/lib.rs:
random boards of every kind of part, the crossings and escape terms on,
check mode off and on on one thread) and the fixture test
`the_place_check_places_alike_in_check_mode` (tests/fixtures.rs:
`place-kicad-board` of F1 both ways, the placed board byte for byte and
board-placer.json's work) hold it; a new check must pass them.

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

## The third router round (2026-10-27)

The two structural costs the second round left: the heap (16 to 21 % of
a route through the inline chain, 39 to 41 % of its pops stale) and the
stamp lists' decoding (`Router::stamp` 2.6 to 9.4 %, up from 1.4 to
5.2 % since the memory round's varint lists). Branch router-profile-3;
snapshots x15180 to x15185 (x15185: the two changes kept, d975ffae on
main edc1d286), against main's x1855 (630799ab) and x1856 (edc1d286; a
placer change, the same routes).

### How

Each candidate as one change against the binary without it: k30-SBC
(the heap) or Castor (the stamps) at 60 s of work, two search threads,
pinned rates, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`,
two pairs on SMT siblings in both orientations (the router's CPU
seconds, `perf stat` cycles and instructions). The machine's load was 20
to 40 and its memory ledger full (the nightly's 4 to 7 GB requests
drained it): a binary that waits in the ledger while the other side of
its pair runs alone gets a whole core and the pair is void (the first
decrease-key pairs: one side waited several minutes; discarded, and the
8-ary heap without prefetch had waits of a minute or two on one side). Each pair now reserves
memory for both binaries at once (`harness_common.admit`, 3 to 4.5 GB)
and starts both inside that reservation (`PCB_MAKER_LEDGER_INSIDE`), so
the ledger delays a pair, never one side of it.

The stamps' profile: the profiling binary (as in "How to profile") on
Castor and Sisu at 60 s of work, the samples of `Router::stamp`
attributed through the inline chain (`perf script -F ip,sym,symoff
--no-inline`, `addr2line -a -i -f` per address; a sample counts as
decoding when a frame is `StampIter::next` or the look-ahead's `Skip`):

| part of `Router::stamp` (% of the route's samples) | Castor | Sisu |
| --- | ---: | ---: |
| all | 10.4 | 8.3 |
| decoding the lists (`StampIter::next`, the look-ahead `skip`) | 3.0 | 2.1 |
| marking the footprints, encoding the new list (`mark_footprint`, `Stamped::push`) | 2.4 | 2.0 |
| the occupancy counts | 1.9 | 1.4 |
| the passes' marks and tile claims | 1.6 | 1.3 |
| the rest (spills, the footprint loops) | 1.5 | 1.5 |

The restamp's two passes each read their list twice: once in order and
once 32 entries ahead (a second iterator, `skip(32)`) for the occupancy
prefetch, so every stamp was decoded twice per pass.

### Candidates measured

| candidate | snapshot | against | board, work | CPU (both orientations) | instructions | identical |
| --- | --- | --- | --- | --- | ---: | --- |
| 8-ary heap (a group of 8 keys a cache line, root slot 7; the 8 grandchild lines prefetched a level) | x15180 | x1855 | k30-SBC 60 s | +1.9 %, +1.4 % | +3.6 % | yes |
| 8-ary heap without the grandchildren's prefetch | x15182 | x1855 | k30-SBC 60 s | +1.8 %, +5.0 % | -0.4 % | yes |
| decrease-key, slots in a separate `Vec<u32>` (4 bytes a state more) | x15181 | x1855 | k30-SBC 60 s | -2.8 %, +1.8 % | -4.1 % | yes |
| decrease-key, the slot in the 12-byte search node (kept) | x15183 | x1855 | k30-SBC 60 s | -4.7 %, -3.2 % | -4.0 % | yes |
| the restamp decodes each list once (kept) | x15184 | x15183 | Castor 60 s | -1.6 %, -2.9 % | -2.9 % | yes |

The 8-ary heap loses either way: a level is a whole cache line and seven
comparisons, the heap a level shallower (log8 against log4 of up to
245 k keys), but the search's pushes fill the root a pop leaves and stop
a level or two down (the first round), where the 4-ary heap's half-line
groups cost less. The radix heap of the second round (+5 %) was not
retried.

The decrease-key with its slots in an array of their own gains less and
less steadily than with the slot in the node: every move of a key in the
heap writes its state's slot, and in the node the new key's own slot is
on the line the step has just written (its cost, parent and marks).

### The changes (bit-identical)

1. **The search's open list with a decrease-key** (`SearchHeap`,
   heap.rs; search.rs). The heap is the 4-ary `KeyHeap` with the empty
   root a pop leaves and the same prefetches, and every move of a key
   writes its slot into its state's `SearchNode::slot`. A step that finds
   a cheaper path to a node already queued moves its key up from that
   slot (`decrease`) instead of queueing a second key, so no pop is stale
   and a pop is an expansion (1.63 to 1.70 pops an expansion before).
   The pops are the lazy heap's without its stale ones: a node's
   heuristic is a function of its state, so a lower cost gives a key no
   greater (`heap_key`: the estimate's bits above the state; float
   addition is monotone), the heap holds each queued node's least key,
   which the lazy heap popped first, and a closed node is never queued
   again (the search does not reopen nodes). Ties resolve as before: a
   key holds its state, equal keys are one entry. The node stays 12
   bytes: the slot (4 bytes) took the closed mark's and the target
   mark's bytes. A node of the search's generation (`seen`) is queued at
   its slot, closed (`SLOT_CLOSED`) or a target not reached yet
   (`SLOT_UNQUEUED`: the targets are marked seen with an infinite cost
   before the search starts, and the first step to one queues it); the
   target mark is the high bit of `since_via` (`TARGET_MARK`), whose
   step count stops at 127 instead of 255 (a lower bound either way:
   where it stops, the hop test walks as before). The `memory:` line,
   which sizes the ladder's routers side by side, counts 12 bytes a node
   as before. Tests: `search_heap_pops_as_a_lazy_heap` (a search's
   rhythm of pops, pushes and decreases, some below the last pop, against
   a `BinaryHeap` that queues every key and skips the expanded states;
   the slots checked), `search_heap_hints_the_next_pop`.
2. **The restamp's passes decode each list once** (`Stamped::iter_ahead`,
   router.rs; stamping.rs): one iterator decodes the list into a ring of
   the 32 pairs ahead, yielding each pair with the one 32 entries on for
   the occupancy prefetch, as the two iterators did. No memory changes
   (256 bytes of ring on the stack). Test:
   `stamps_ahead_pair_each_stamp_with_the_one_ahead`.

Not taken for the stamps: a per-router cache of decoded lists (8 bytes
a stamp against the varints' 1.5: the memory round's saving), group or
fixed-width deltas (the lists' bytes and the decode change; with one
decode a pass left, 1 to 1.5 % of a route is all there is to gain).

### Measured (2026-10-27)

Snapshots x1856 (main edc1d286) and x15185 (d975ffae, the two changes
on it), side by side on SMT siblings (one binary on two of the CPUs 4 to
15, the other on their siblings 16 higher; the second pair with the
binaries' CPUs swapped, on the next two cores), each pair inside one
reservation of the ledger, two search threads, pinned rates,
`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1` (and
`PCB_ROUTER_MEM_PROFILE=1` on Castor and k30-SBC), the four fingerprint
boards at a quarter budget (v16 placements), SmartSpin2k and protoesp
at the task budget (v19), the machine's load 20 to 40. Cells: the two
pairs' CPU seconds of the router process (x1856's first orientation
first); the change, the geometric mean of the two pairs' ratios;
cycles and instructions (`perf stat`, the same mean); million
expansions per CPU second; the route's expansions and searches.

| board | budget | x1856 CPU s | x15185 CPU s | CPU | cycles | instructions | x1856 M exp. / CPU s | x15185 | expansions | searches | identical |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Castor | 300 | 425 / 433 | 416 / 417 | -3.0 % | -3.0 % | -6.9 % | 1.02 / 1.00 | 1.04 / 1.04 | 434 M | 8267 | yes (2 pairs) |
| jetson | 300 | 596 / 616 | 588 / 608 | -1.3 % | -1.7 % | -5.7 % | 1.01 / 0.98 | 1.03 / 0.99 | 604 M | 10416 | yes (2 pairs) |
| k30-SBC | 300 | 589 / 551 | 573 / 537 | -2.7 % | -2.9 % | -5.4 % | 0.84 / 0.90 | 0.87 / 0.93 | 497 M | 8758 | yes (2 pairs) |
| Sisu | 300 | 702 / 699 | 669 / 677 | -3.9 % | -4.1 % | -7.2 % | 0.87 / 0.88 | 0.92 / 0.91 | 613 M | 10012 | yes (2 pairs) |
| SmartSpin2k | 1200 | 1219 / 1146 | 1153 / 1080 | -5.6 % | -5.8 % | -9.4 % | 1.78 / 1.89 | 1.88 / 2.01 | 2166 M | 32073 | yes (2 pairs) |
| protoesp | 1200 | 1298 / 1273 | 1191 / 1206 | -6.8 % | -6.7 % | -9.3 % | 0.62 / 0.64 | 0.68 / 0.67 | 810 M | 11491 | yes (2 pairs) |

The CPU falls on every board, 1.3 to 6.8 % (-3.9 % as the geometric
mean of the six; the single pairs -1.3 to -8.2 %), most on the dense
boards at the task budget, whose searches are long and whose heaps are
large; instructions fall 5 to 9 %.

**Identical** in every pair: the routed board's SHA-256, every
fingerprint line, the expansions and searches, and verdict.json apart
from its work, wall, rate, memory and path fields. The expansions and
searches are the second round's on every board, and Castor's board is
still the first round's (SHA-256 prefix dad3a6cb3787b575). The
expansion tier (46 boards, 8 x 1, 30 s): 46 of 46 routed boards
byte-identical to x1856's (another package's run of the tier, on the
same day), the rows identical apart from their timing fields and the
intake's speed class (`outcome`, from the route's seconds: three
clean_medium to clean_slow, one clean_slow to clean_medium), 557
unconnected, 0 new errors, 4263 vias both. The tests
(`scripts/check.sh --fast`, clippy with warnings as errors) pass.

**Peak resident size** (`PCB_ROUTER_MEM_PROFILE=1`, `peak rss`, both
pairs): Castor 602 and 606 MB (x1856) against 602 and 617 MB (x15185),
k30-SBC 904 and 892 MB against 885 and 910 MB; at the peak (the probes)
the search arrays 123 to 126 MB on Castor either way. The memory round's
saving (Castor 1073 to 661 MB, k30-SBC 1630 to 1000 MB at the time) stands:
the node stays 12 bytes and the open list holds no stale keys.

**The work clock.** Pinned rates (every tier, run.py) do the same
expansions in less CPU. The canned search of an unpinned rate
(crates/pcb-kicad/src/work_rate.rs) got faster too: six calibrations side
by side on one CPU and its sibling (`calibrate-work-rate 1`, alternating)
gave medians of 2.226 M (x1856) and 2.273 M (x15185) expansions a
second, the pairs' ratios 1.009 to 1.034: an unpinned run measures a rate
about 2 % higher and spends that much more search in the same budget,
so its results differ from x1856's as with any change of the measured
rate.

### What is next

- The heap is still the search's largest part beside the steps: each
  move of a key writes its state's slot (a store to another node's
  line). A push into the empty root that ends a level or two down moves
  one or two keys; whether a lazier fill of the root (taking the least
  child only at the next pop) saves stores is untried.
- The stamps: one decode a pass is left (1 to 1.5 % of a route on the
  pour boards), and the marking of the footprints with its varint
  encoding (2 to 2.4 %). Encoding into a reserved buffer without
  `Vec::push`'s capacity test a byte would be the next small step.
- The steps' counts and the prefetch of the next pop's rows (the second
  round's list) remain.

## The fourth router round (2026-11-04)

What the router gained since the third round (2026-10-27) without a CPU
profile: the length repair after the clean-up, the neck-gap repair with
its bridges and extra KiCad checks, graphic-stub filtering in
`emit_routes`, the trouble repair's extra hard reroute of one-sided
conflicts, vias in BGA pads (the via-in-pad plan at every lowering),
pin-row gaps rebuilt on every layout move, and the race's router copy
per probe (the ripped count). Branch router-profile-4: five code
commits and this note, measured on main 951b610a (x1889; x1888 to x1891
route as x1882), the head built there as
`build/bin/pcb-maker-rprof4-final`, then rebased on main 4eed451a (no
router change between); each commit's binary before the rebases
`build/bin/pcb-maker-rprof4-k1` to `-k5`; outputs in `build/rprof4/`.

### How

perf was not installable again. Three ways, as the placer rounds:

- **Categories** (a throwaway build, branch router-profile-4-attr, not
  for merging: `pcb_router::attr`, thread CPU time per category from
  `CLOCK_THREAD_CPUTIME_ID` around a function or a block, summed over
  the threads and printed at the end with `PCB_ROUTER_ATTR`): the search,
  the stamping, the negotiation, the hard repair, the trouble repair and
  its parts (the priced search, the victim lists, the victims' reroutes,
  the extra reroute of one-sided conflicts, the saved states), the
  clean-up, the length repair (and its victim list), the via reduction,
  the spoke repairs, the straightening, the stitching, the manual
  spokes, the lowering (and in it the via-in-pad plan), `Router::new`,
  `mark_pin_rows`, `Router::update`, the race's copies and
  `rip_up_standing`, `emit_routes` and `graphic_stubs`, the attempt's
  write, the meanders, the verification, the board's parse, the final
  pour measure (`measured_board`, and in it the parse, the lowering,
  `Router::new`, the adoption and the report), the neck repair (CPU and
  wall) and every KiCad check's wall; a second and third build split the
  pour analysis (the free mask, the cuts, the labelling, the solid pads'
  windows, `kicad_spokes`, the refresh). The categories nest (a search
  inside the trouble repair is in both); shares of the process's CPU.
- **Callgrind** (valgrind 3.25) on short routes (10 s of work, one
  thread, pinned rates, `PCB_ROUTER_NO_WALL_GUARD=1`): nrf24_remote
  with the collection toggled on `analyze_pours_uncounted`, before and
  after the window change; pdlc whole, and its `Router::stamp` with the
  profiling build of "How to profile" for the lines; brickpico's
  analyses after the changes.
- **Pinned pairs**: route mode on the four fingerprint boards (v16
  placements, a quarter budget: 300 s of work, two search threads) and
  the expansion tier's eight heaviest boards (the nightlies of
  2026-11-01 to 03 by the median CPU: node_v6_0_0, brickpico,
  anthracite-bottom, the power-manifold backplane, nrf24_remote, Wonkle,
  pdlc, the MusicBrain busboard; their nightly sources and router
  settings, 30 s of work, one search thread, the intake's 90 s wall
  deadline left out so that no wall cut can differ); layouts of four
  dense boards (protoesp, SmartSpin2k_Panelized, MegaDrive, 4in1-mini;
  the placer rounds' task sources, seed 1, two threads). Each pair: the
  off binary on two CPUs, the on binary on their SMT siblings, started
  together inside one reservation of the memory ledger for both
  (`PCB_MAKER_LEDGER_INSIDE`, the boards' measured estimates), pinned
  rates, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`; the
  second orientation (CPUs swapped) at the same time on two other cores;
  the router process's own CPU (utime + stime at its exit, kicad-cli
  left out). The harness is `build/rprof4/pairs.py` (scratch).

### Where a route's CPU goes (main 688b59ca, the categories)

Route mode, one run per board under the timing build (unpinned, the
machine's load 24 to 38; shares of the router process's CPU, the
categories nested as the code nests them; "-" where the category did
not run):

| category (% of the route's CPU) | Castor | k30-SBC | jetson | Sisu | node_v6 | brickpico | anthracite | backplane | nrf24 | Wonkle | pdlc | busboard |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| route CPU s (the timing build) | 314 | 344 | 580 | 515 | 87 | 81 | 58 | 61 | 81 | 63 | 46 | 52 |
| search | 77.2 | 83.5 | 76.8 | 76.2 | 30.8 | 48.4 | 56.3 | 65.6 | 50.9 | 61.2 | 62.0 | 51.1 |
| stamping (`stamp`, `unstamp`) | 7.5 | 4.9 | 2.6 | 9.4 | 1.4 | 6.2 | 12.3 | 3.8 | 4.8 | 5.8 | 20.2 | 10.1 |
| pour analyses (the log's seconds) | 5.9 | 0.0 | 0.0 | 0.0 | 0.0 | 28.1 | 0.0 | 0.0 | 32.6 | 6.8 | 3.5 | 22.3 |
| hard repair (`resolve_remaining`) | 0.0 | 1.3 | 4.7 | 12.3 | 7.9 | 0.0 | 0.0 | 2.9 | 2.4 | 27.0 | 3.3 | 2.2 |
| trouble repair | 0.0 | 0.0 | 3.0 | 4.9 | 0.0 | 0.0 | 0.0 | 1.9 | 0.6 | 0.0 | 0.9 | 0.0 |
| of it: priced searches | - | - | 0.3 | 0.3 | - | - | - | 0.3 | 0.2 | - | 0.3 | - |
| of it: victim lists | - | - | 0.01 | 0.00 | - | - | - | 0.00 | 0.00 | - | 0.00 | - |
| of it: the victims' reroutes | - | - | 2.6 | 4.4 | - | - | - | 1.5 | 0.3 | - | 0.3 | - |
| of it: the extra reroute (one-sided conflicts) | - | - | 0.00 | 0.00 | - | - | - | 0.00 | 0.00 | - | 0.00 | - |
| clean-up | 27.5 | 14.5 | 8.1 | 8.4 | 0.0 | 22.7 | 15.4 | 17.1 | 29.9 | 0.9 | 37.1 | 30.4 |
| length repair | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 |
| via reduction | 25.6 | 21.9 | 0.0 | 0.0 | 0.0 | 38.5 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 |
| spoke repairs, straightening, stitching, manual spokes | 7.4 | 1.8 | 0.2 | 0.5 | 0.2 | 1.1 | 6.0 | 6.2 | 9.6 | 11.2 | 3.2 | 4.1 |
| lowering (`lower`) | 0.04 | 0.02 | 7.3 | 1.7 | 44.6 | 0.13 | 0.36 | 0.26 | 0.08 | 0.38 | 0.12 | 0.14 |
| via-in-pad plan (in the lowering, the reports, the measure) | 0.00 | 0.00 | 9.5 | 2.4 | 58.9 | 0.01 | 0.05 | 0.25 | 0.03 | 0.08 | 0.02 | 0.02 |
| pin-row gaps (`mark_pin_rows`) | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 |
| `emit_routes` (with `graphic_stubs`) | 0.00 | 0.00 | 0.01 | 0.00 | 0.01 | 0.00 | 0.03 | 0.01 | 0.06 | 0.06 | 0.05 | 0.04 |
| the attempt's write (encode, the report's plan) | 0.02 | 0.01 | 1.2 | 0.4 | 6.0 | 0.02 | 0.17 | 0.06 | 0.27 | 0.28 | 0.28 | 0.11 |
| the final pour measure (`measured_board`) | 0.4 | 0.3 | 1.6 | 2.3 | 8.9 | 1.7 | 7.4 | 3.5 | 0.1 | 6.3 | 0.7 | 5.1 |
| verification, the board's parse | 0.06 | 0.02 | 0.9 | 0.1 | 0.2 | 0.1 | 0.4 | 0.1 | 0.4 | 0.8 | 0.4 | 0.2 |

- **The via-in-pad plan** (vias in BGA pads, 2026-10-28) is the largest
  new cost: `plan` runs at every lowering (each rung, each attempt), for
  the report of each board written (`finish_routed_board`, on the board
  without its vias) and in the final measure, and judges every
  footprint's enclosed pads (`enclosed_pads`: a flood of the pad field
  on a raster of 5 to 25 microns, once per net of the candidates)
  whether the rule admits it or not, for the report's closed pads.
  node_v6_0_0: 59 % of its route (the lowering 45 %, the write 6 %, the
  measure 9 %), jetson 9.5 %, Sisu 2.4 %, protoesp's layout 2 %.
- **The pour analyses** (not new, the pour boards' second largest
  cost): nrf24_remote 33 to 37 %, brickpico 28 to 34 %, the busboard 22
  %. Callgrind (nrf24_remote, 10 s of work, the analyses only): the
  window scans around the solid pads' nodes (`pieces_near`, 38 % of the
  analyses' instructions) and around every pad's nodes for the spokes'
  items (`piece_near` from `kicad_spokes`, 27 %): a pad's hundreds of
  nodes at a 0.05 mm pitch each looked at the 33 x 33 nodes of its
  window; the labelling (`PourMap::build`, 14 %; a tenth of brickpico's
  and the busboard's routes), the free mask (7 %), the fill probe (5 %).
- **The rest is small**: the trouble repair 0 to 5 % (its victim lists
  0.01 % at most, the extra reroute below the timer's resolution; the
  victims' reroutes are searches), the length repair 0 (no board here
  has a length rule; oro's layout below), `mark_pin_rows` at most 3 ms a
  run, `emit_routes` and `graphic_stubs` at most 0.06 %, the race's
  copies 0.01 to 0.14 s and `rip_up_standing` 0.24 to 0.50 s a layout
  (protoesp, SmartSpin2k, 4in1-mini, USB_Keypad), `Router::update` 1 to
  6 s a layout.
- **Stamping** 2.6 to 20 % (pdlc's seven classes: each stamp marks its
  footprints once per querying class): callgrind's lines of
  `Router::stamp` spread over the varint decode (15 %), the look-ahead
  ring (8 %), the marks (10 %), the encode (6 %) and the counts: no
  single line to take.
- **The final pour measure** (`measured_board`, counted as no work): on
  anthracite 6.0 s, of it `Router::new` 3.4 s (two builds of the full
  lattice: the measure's board is the written one, its tracks fixed
  copper, so the route's held static maps do not fit it) and the report
  2.4 s.

**Layouts** (the timing build, the layout's budget, seed 1, two threads;
shares of the layout process's CPU; seconds where marked):

| category (% of the layout's CPU) | protoesp | SmartSpin2k | MegaDrive | 4in1-mini | USB_Keypad | WLED_Matrix | oro link |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| layout CPU s | 1719 | 1522 | 1916 | 1274 | 879 | 1349 | 3162 |
| search | 79.2 | 85.8 | 66.9 | 61.6 | 49.4 | 50.8 | 56.9 |
| stamping | 2.9 | 1.6 | 7.3 | 2.5 | 5.5 | 6.1 | 8.8 |
| hard repair | 17.8 | 26.5 | 10.1 | 10.5 | 1.7 | 3.7 | 3.8 |
| trouble repair (of it the victims' reroutes) | 1.0 (0.3) | 0.5 (0.4) | 3.3 (1.7) | 6.4 (4.4) | 0.3 (0.1) | 0.7 (0.4) | 0.2 (0.3) |
| pour analyses | 1.3 | - | 1.6 | 1.6 | 5.1 | 7.4 | 12.7 |
| lowering / via-in-pad plan | 1.9 / 2.1 | 0.1 / 0.1 | 0.0 / 0.0 | 0.0 / 0.0 | 0.4 / 0.4 | 0.3 / 0.3 | 0.2 / 0.2 |
| length repair, meanders (s) | - | - | - | 0, 0 | 0, 0 | 0, 0 | 12.6, 0.3 |
| the race's copies, `rip_up_standing` (s) | 0.07, 0.35 | 0.14, 0.27 | 0.38, 5.16 | 0.03, 0.50 | 0.01, 0.24 | 0.02, 0.34 | 0.80, 3.37 |
| `Router::update`, the moves' rebuilds (s) | 6.1 | 1.3 | 7.0 | - | 1.0 | 2.1 | 96.6 |
| the neck repair: router CPU s / wall s | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 5.3 / 326 | 4.9 / 291 | 0 / 0 |
| KiCad's checks, wall s | - | 13 | - | - | 345 (24 checks) | 324 | 22 |

The layouts' CPU is the search and the placement (not timed here: the
placer rounds'). The length repair is oro's 0.4 %, the meanders 0.01 %;
the moves' rebuilds (`Router::update`, with the pin-row gaps since
2026-11-03: `mark_pin_rows` 1 to 3 ms a layout) 3 % of oro's layout,
below 0.4 % elsewhere; the ripped count's copies and rip-ups 0.3 % of
MegaDrive's at most (main's x1889 since picks the rip-up from a heap).
The neck repair's cost is KiCad's: USB_Keypad's 24 checks took 345 s of
wall clock (kicad-cli, a child: not the router's CPU) against 5.3 s of
the router's own (the board's parse, the lowering, the trials'
encodings; the raster `find_necks` 1 ms). Its
trials are sequential by construction (each half's check carries the
tracks the halves before it kept), so they cannot run side by side and
keep the same checks.

### The changes (bit-identical)

Five commits, each the same computation: the same answers in the same
order, the same work booked (the pour analyses book their model's
units, the fill probe its questions, neither changed), each with a unit
test against the direct computation.

1. **The via-in-pad plan's enclosed pads kept by their exact inputs**
   (via_in_pad.rs `enclosed_of`, `enclosed_key`, `ENCLOSED`). A
   footprint of nine pads or more (fewer hold no enclosed pad) is judged
   once per process for the same inputs: the key is every word
   `enclosed_pads` reads (each pad's number, centre, shape, layers,
   kind, net and clearance as bits, and each net's track, clearance and
   via), compared whole; the answer is kept as positions among the
   footprint's pads. A layout's moves make every pose of a large part a
   new key: the cache is emptied at 64 MB of keys. Test
   `the_enclosed_pads_are_kept_by_their_exact_inputs` (the cached
   answers against `enclosed_pads`, a ball moved by a nanometre and
   rules narrow enough to pass between the balls are other keys with
   other answers).
2. **The pour analysis's windows nearest first** (pour.rs `NearOrder`,
   `piece_near_ordered`, `pieces_near_ordered`, `pieces_in_box`). A
   window's nodes in the order of their distance, ties in the window's
   row-major order: the first labelled node met is `piece_near`'s
   answer (the scan kept a strictly nearer node, so of the nearest the
   first in row-major order), each piece's first node its nearest in
   `pieces_near`'s sense; the spokes' items stop at the first, the
   solid pads' windows once every piece the box of the pad's windows
   holds was met. Tests `the_ordered_windows_find_what_the_scans_find`
   (four label maps, reaches 0 to 16, every node, against the scans),
   `the_order_is_nearest_first_then_row_major`.
3. **The free mask reads its map's bits once** (`OwnStamps::bits_of`):
   the slot lookup and its assertion once a layer instead of once a
   node.
4. **The pour map labelled by runs** (`PourMap::build`). The flood
   node by node became runs of solid nodes along the rows, joined to
   the runs above that overlap them (side by side) and to the run above
   that ends just before a run's start or begins just after its end
   (diagonally: there both common neighbours are free of the pour by
   construction, and they are the only diagonal steps the flood asked
   `diagonal` about); a piece is numbered at its first run in the
   lattice's order, where the flood started it. Test
   `the_pieces_by_runs_are_the_floods` (seven maps, a deterministic half
   of the diagonal steps allowed and all of them, against the flood,
   kept as the test's reference).
5. **A solid pad's windows look only for the pieces that can join it**.
   A piece's candidate is its node nearest the pad among the nodes'
   windows' nearest; `solid_fill_reaches` refuses a candidate farther
   than the brush and two pitches before it asks the probe anything, so
   only pieces with a node within that of the pad can join it: the
   windows look for the pieces in the pad's box grown by the brush and
   three pitches (none: no window at all), and `pieces_near_ordered`
   keeps the pieces it is given. Test: subsets of the pieces against the
   full answer.

Not taken: the stamp lists (no line above 5 % of `Router::stamp`; the
third round's notes stand), the final pour measure's two lattice builds
(the written board's tracks are fixed copper there: the route's held
maps do not fit; a third of anthracite's 6 s), the work-rate
calibration at the start of every run with pinned rates (8 % of pdlc's
instructions at 10 s of work, a few CPU seconds a run: the measured
rate goes into the report and the wall guard's stretch, so dropping it
changes what a run reports), the trouble repair's victim lists and the
race's copies (below a hundredth of a percent and 0.2 s a layout), the
neck repair's checks (KiCad's wall clock, sequential by construction).

### Measured (2026-11-04)

**Route mode** (pinned pairs as in "How": x1889, main 951b610a,
against the head, `build/bin/pcb-maker-rprof4-final`, on 951b610a; the
two orientations each its own series of reservations, one after the
other on CPUs 0, 1 and 8, 9 and their siblings; the machine's load 8 to
45; 04:25 to 07:34). Cells: each pair's CPU seconds of the router
process (x1889 on the first CPUs first, then swapped); the change, the
geometric mean of the pairs' ratios; peak resident size (MB); the
routed board and the fingerprint lines:

| board | budget | x1889 CPU s | head CPU s | CPU | x1889 MB | head MB | identical |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Castor | 300 | 340.2 / 414.0 | 314.8 / 404.1 | -5.0 % | 600 / 602 | 591 / 598 | yes (2 pairs, 6 fingerprint lines) |
| k30-SBC | 300 | 445.6 / 513.7 / 515.3 / 428.0 | 461.5 / 508.7 / 532.0 / 482.1 | +4.5 % | 908 / 889 / 893 / 892 | 890 / 896 / 888 / 894 | yes (4 pairs, 5 fingerprint lines) |
| jetson | 300 | 620.4 / 514.6 | 583.1 / 507.2 | -3.8 % | 1207 / 1213 | 1210 / 1216 | yes (2 pairs, 5 fingerprint lines) |
| Sisu | 300 | 724.1 / 531.7 | 700.5 / 504.0 | -4.2 % | 1424 / 1422 | 1425 / 1432 | yes (2 pairs, 5 fingerprint lines) |
| node_v6_0_0 | 30 | 127.3 / 57.1 | 64.4 / 33.6 | -45.5 % | 356 / 366 | 342 / 342 | yes (2 pairs, 5 fingerprint lines) |
| brickpico | 30 | 97.1 / 95.8 | 81.2 / 87.9 | -12.4 % | 524 / 526 | 533 / 526 | yes (2 pairs, 6 fingerprint lines) |
| anthracite-bottom | 30 | 79.5 / 83.6 / 63.1 / 82.2 | 80.1 / 82.0 / 68.0 / 80.9 | +1.2 % | 491 / 496 / 484 / 490 | 492 / 511 / 489 / 492 | yes (4 pairs, 5 fingerprint lines) |
| backplane | 30 | 81.3 / 68.2 | 76.5 / 68.7 | -2.7 % | 381 / 372 | 369 / 371 | yes (2 pairs, 5 fingerprint lines) |
| nrf24_remote | 30 | 79.1 / 79.5 | 55.1 / 54.9 | -30.7 % | 134 / 135 | 134 / 138 | yes (2 pairs, 71 fingerprint lines) |
| Wonkle | 30 | 73.0 / 78.8 | 68.2 / 76.5 | -4.7 % | 592 / 590 | 590 / 586 | yes (2 pairs, 6 fingerprint lines) |
| pdlc | 30 | 66.9 / 64.1 / 64.6 / 63.7 | 61.5 / 60.5 / 64.7 / 62.6 | -3.9 % | 252 / 260 / 254 / 257 | 263 / 258 / 237 / 260 | yes (4 pairs, 59 fingerprint lines) |
| musicbrain-busboard | 30 | 76.9 / 61.0 | 61.1 / 49.0 | -20.0 % | 473 / 472 | 474 / 471 | yes (2 pairs, 6 fingerprint lines) |

Over the twelve boards the router's CPU falls 11.9 % as the geometric
mean (the eight expansion boards 16.4 %, the four fingerprint boards
2.2 %). Where the profile pointed: node_v6_0_0
-45 % (its plan), nrf24_remote -31 % and the busboard -20 % (the
windows, the labelling), brickpico -12 % (the labelling; its windows
still a large share, below), Castor -5 %, jetson -4 % (the plan), Sisu
-4 %. Where little it runs changed, the pairs give the noise:
backplane -2.7 %, Wonkle -4.7 %, pdlc -3.9 % and anthracite +1.2 % (four
pairs each for these two: +0.8, -1.9, +7.8, -1.6 %), and k30-SBC
+4.5 % (four pairs: +3.6, -1.0, +3.2, +12.6 %), though callgrind counts
5.7 % fewer instructions for k30-SBC's route on the head at 10 s of work
(190 G against 179 G: the pour analyses 22.2 G to 12.2 G, the search
105.5 G on both): its CPU in the pairs is the pairs' spread (the third
round's jetson orientations: -2.4 and -15.1 %) or the code's placement,
not more work.

**Layouts** (`layout-kicad-board`, the task's budget, seed 1, two
threads, one orientation a reservation; the layout's CPU, the
placement's to `layout: placed`, the rest from there; peak RSS; the
placed and the result boards and the fingerprint lines):

| board | x1889 CPU s | head CPU s | CPU | x1889 placement | head | placement | x1889 routing | head | routing | x1889 MB | head MB | identical |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| protoesp | 1904.7 / 2252.8 | 1845.9 / 2202.6 | -2.7 % | 309 / 370 | 303 / 368 | -1.2 % | 1596 / 1882 | 1543 / 1834 | -2.9 % | 411 / 415 | 404 / 394 | yes (2 pairs, 35 fingerprint lines) |
| SmartSpin2k_Panelized | 1642.4 / 1658.0 | 1609.6 / 1622.7 | -2.1 % | 214 / 242 | 213 / 238 | -1.1 % | 1429 / 1416 | 1396 / 1385 | -2.2 % | 1217 / 1221 | 1216 / 1222 | yes (2 pairs, 8 fingerprint lines) |
| MegaDrive | 2439.4 / 2231.9 | 2371.4 / 2238.1 | -1.3 % | 395 / 379 | 390 / 382 | -0.3 % | 2044 / 1852 | 1981 / 1856 | -1.5 % | 2016 / 2021 | 2019 / 1995 | yes (2 pairs, 20 fingerprint lines) |
| 4in1-mini | 1531.8 / 1000.9 | 1520.8 / 1010.1 | +0.1 % | 450 / 362 | 448 / 379 | +2.1 % | 1082 / 639 | 1073 / 631 | -1.0 % | 257 / 262 | 257 / 259 | yes (2 pairs, 11 fingerprint lines) |

The layouts move by +0.1 to -2.7 % (-1.5 % as the geometric mean of the
four): their CPU is the search and the placement, which this round did
not change. The routing part (from `layout: placed` on) falls 1.0 to
2.9 % on every board (seven of the eight pairs), most on protoesp (its
via-in-pad plan was 2 % of its layout, and its pours); the placements
move by -1.2 to +2.1 % with nothing of the placer changed: the pairs'
spread.

**Identical.** Every pair above: the routed (or placed and result)
board's SHA-256 and every fingerprint line (as a sorted set). Each
commit on its own (`-k1` to `-k5`, built before the last rebases, the
same router code), unpinned, against x1888 (which routes as x1889) on
node_v6_0_0, nrf24_remote, brickpico, the busboard, Wonkle and
anthracite: the same boards and fingerprint lines from all six
binaries. The expansion tier (46 boards, route mode, 8 x 1, 30 s,
`run.py --tier expansion` with the head) against the nightly of
2026-11-04 (main 688b59ca, the same router): 46 of 46 routed boards
byte-identical, the rows' routed counts, vias, KiCad findings, passes
and verdicts identical, 558 unconnected, 24 strict passes, 4256 vias
both, no wall cut (its CPU 1735 against the nightly's 2318 s: unpinned,
other loads; no measurement). `scripts/check.sh --fast` passes (clippy
with warnings as errors).

**The work clock**: unchanged by construction. The cache, the windows
and the labelling compute what was computed before; the pour analyses
book their model's units (nodes inside the pour, stamps) and the fill
probe its questions, both the same.

### What is next

- **brickpico's pour analyses** after the changes (callgrind, 10 s of
  work, the analyses only): the solid pads' windows are still 45 % of
  its analyses (its pads have pieces near them that their inner nodes'
  windows lack, so the windows are walked to their end), `kicad_spokes`
  27 %, the free mask 12 %. An exact bound on where a window's nearest
  node of a piece can lie closer than the brush and two pitches to the
  pad would let the walk stop early.
- **The search** is again 50 to 85 % of every route and layout (74 % of
  nrf24_remote's route after the changes); the third round's list
  stands.
- **Stamping** (2 to 20 %; pdlc's seven classes): spread over the varint
  decode, the ring, the marks and the encode.
- **The final pour measure's lattice** (anthracite 3.4 s of 6 s): a
  build of the written board's maps, which nothing shares.

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

## The second placer round (2026-10-28)

The placement side of a layout again, after the layout round: the
placement (`place_kicad_board`: the race's seeds, each a global
placement, anneal, legalisation and refinement with its repairs), the
first part of the layout's budget (40 %). Branch placer-profile-2:
eleven commits measured on main e54c6ada (x1871, the anneal's crossings
term on by default since that commit), the head
built as `build/bin/pcb-maker-prof4-final` (3daff389), each commit's
binary as `build/bin/pcb-maker-prof4-n1` to `-n10`; rebased on main
5905bf9a (x1872) as 55235df8 to 28119348, and a twelfth commit (37b81380,
a debug switch's work, below).

### How

perf could not be fetched this round (the package download was refused
in the agent's sandbox), so the placement was attributed three ways:

- **Phases** (`PCB_PLACER_DEBUG`): each phase's thread CPU (global,
  anneal, the legalisations, refinement, repair and alignment), summed
  over the seeds; they add up to the process's CPU within 0.1 to 2 %.
- **Inside the phases**: a throwaway build (branch
  placer-profile-2-attr, not for merging: `attr.rs`, a time-stamp
  counter per category and thread, printed at the end of each seed with
  `PCB_PLACER_ATTR`) times the anneal's wirelength, relation, overlap
  and crossings terms, the legaliser's legality tests and escape term,
  the refinement's wirelength, relation penalty, escape term, escape
  measurements (the global route), crossings, legality and equal-part
  swaps, and the relation repair's legality tests; each phase's CPU is
  split by its categories' share of the phase's ticks.
- **Callgrind** (valgrind 3.25, `--tool=callgrind`, the profiling build
  of "How to profile") on short placements (`PCB_PLACER_WORK_SECONDS`
  15 to 30, one thread) of Castor, Glasgow, MegaDrive, SmartSpin2k,
  DSP-ADAU1452 and protoesp, for the functions inside a category; the
  shares at a short budget are not the full placement's (the seeds'
  repairs come late), the categories above are.

`place-kicad-board <task source> <board> <out> <layout.json> --router
auto` is the layout's placement byte for byte since the third trial's
follow-ups (the check takes the layout's budget share, the race's seeds,
the router's rules for the escape measurements and pins the reference's
rates), so the placement was measured alone: the eleven dense boards'
task sources (tasks v7, seed 1, as run.py prepares them) and the four
fingerprint boards' (Castor, k30-SBC, jetson, Sisu), two threads on two
CPUs of one core (`taskset`), `PCB_ROUTER_NO_WALL_GUARD=1`, pinned rates.

### Where the placement's CPU goes (main e54c6ada, x1871)

Shares of the placement's CPU (the process's, two threads):

| category (% of the placement's CPU) | 0xCB panel | protoesp | Glasgow | video | 4in1-mini | 4in1 | SmartSpin2k | DSP | MegaDrive | SNSP | split72 | Castor | k30-SBC | jetson | Sisu |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| placement CPU s (2 threads) | 209 | 1001 | 1579 | 532 | 855 | 1529 | 904 | 616 | 664 | 799 | 727 | 258 | 244 | 596 | 685 |
| global placement | 17.1 | 2.6 | 0.2 | 1.5 | 0.5 | 0.2 | 1.9 | 0.2 | 0.1 | 0.5 | 0.3 | 2.8 | 1.5 | 0.4 | 0.5 |
| anneal: wirelength | 3.8 | 1.3 | 3.8 | 3.4 | 1.8 | 2.7 | 0.5 | 11.6 | 2.6 | 7.3 | 13.1 | 10.7 | 10.8 | 7.4 | 13.3 |
| anneal: relations, hard constraints | 0.7 | 0.4 | 5.9 | 2.3 | 0.2 | 0.9 | 0.4 | 1.4 | 1.1 | 4.1 | 6.1 | 6.1 | 11.5 | 2.7 | 5.7 |
| anneal: overlap (grid query, pairs) | 7.8 | 6.3 | 4.4 | 1.8 | 10.9 | 10.3 | 0.7 | 32.3 | 22.9 | 23.0 | 5.8 | 17.7 | 6.6 | 15.1 | 16.6 |
| anneal: crossings term | 2.0 | 2.3 | 5.7 | 4.8 | 3.1 | 4.8 | 0.6 | 10.8 | 12.1 | 14.6 | 8.8 | 17.0 | 15.6 | 14.2 | 18.8 |
| anneal: the rest (moves, snap, grid upkeep) | 2.0 | 1.4 | 1.6 | 2.0 | 1.2 | 1.8 | 0.4 | 5.1 | 2.8 | 5.8 | 3.3 | 9.1 | 7.7 | 5.6 | 6.7 |
| legaliser: legality tests | 34.6 | 59.1 | 7.5 | 8.9 | 19.5 | 33.6 | 0.4 | 1.1 | 19.0 | 1.0 | 0.4 | 1.2 | 0.4 | 8.4 | 0.2 |
| legaliser: escape term | 0.0 | 0.0 | 0.8 | 0.9 | 0.7 | 0.5 | 0.1 | 0.0 | 0.1 | 0.0 | 0.0 | 0.1 | 0.1 | 0.1 | 0.0 |
| legaliser: the rest (ring scan, neighbour queries, crossings) | 13.9 | 22.6 | 13.8 | 68.5 | 6.0 | 12.0 | 1.6 | 0.7 | 11.3 | 1.4 | 2.5 | 1.3 | 4.7 | 1.5 | 0.3 |
| refinement: wirelength | 1.7 | 0.3 | 10.4 | 0.0 | 8.5 | 8.1 | 5.5 | 5.4 | 0.8 | 2.2 | 7.3 | 4.3 | 4.7 | 5.3 | 8.2 |
| refinement: relation penalty | 0.2 | 0.1 | 21.8 | 0.0 | 1.2 | 3.2 | 2.8 | 1.0 | 0.5 | 2.1 | 4.7 | 3.1 | 6.3 | 2.9 | 4.7 |
| refinement: escape term | 0.0 | 0.0 | 8.1 | 0.0 | 19.9 | 11.0 | 18.4 | 4.7 | 0.6 | 3.2 | 4.9 | 6.8 | 5.2 | 6.7 | 6.6 |
| refinement: escape measurements (global route) | 0.0 | 0.0 | 0.1 | 5.1 | 0.1 | 0.0 | 0.1 | 0.7 | 11.4 | 3.9 | 0.8 | 1.9 | 1.7 | 2.5 | 1.3 |
| refinement: crossings | 0.5 | 0.1 | 6.7 | 0.1 | 2.1 | 0.6 | 6.2 | 0.4 | 1.8 | 2.3 | 6.1 | 0.9 | 1.1 | 1.7 | 0.9 |
| refinement: legality (neighbour query, test) | 5.7 | 1.7 | 7.8 | 0.0 | 22.5 | 7.0 | 7.7 | 7.5 | 6.6 | 6.2 | 3.7 | 3.5 | 1.8 | 11.0 | 5.4 |
| refinement: equal-part swaps | 0.3 | 0.5 | 0.3 | 0.1 | 0.7 | 0.4 | 0.2 | 7.7 | 1.8 | 22.5 | 29.7 | 7.4 | 1.5 | 10.3 | 3.8 |
| relation repair: legality | 7.9 | 1.1 | 0.4 | 0.0 | 0.0 | 2.2 | 49.8 | 9.1 | 4.4 | 0.6 | 1.7 | 5.0 | 17.4 | 3.6 | 6.5 |
| refinement and repair: the rest | 0.1 | 0.0 | 0.5 | -0.0 | 1.0 | 0.6 | 1.3 | 0.1 | 0.0 | -0.9 | 0.1 | 0.0 | 0.3 | -0.3 | 0.2 |
| outside the phases (lowering, set-up) | 1.6 | 0.3 | 0.1 | 0.2 | 0.2 | 0.2 | 0.5 | 0.3 | 0.2 | 0.2 | 0.5 | 0.9 | 0.7 | 0.7 | 0.2 |

- **No single hot spot**: the anneal's overlap sums (up to 32 %), the
  legaliser's legality tests (up to 59 %), the refinement's escape term
  (up to 20 %), the relation repair's legality tests (up to 50 %), the
  equal-part swaps (up to 30 %) and the anneal's crossings term (2 to
  19 %, on by default since e54c6ada) each dominate a few boards.
- **video's legalisation (69 %)** is its ring search: a part without a
  spot scanned every step of every ring of the board for the steps on the
  ring (`(2 r + 1)^2` a ring, the legality test run only on the ring),
  the "rest" of the legaliser beside the tests.
- **The relation repair** (SmartSpin2k 50 %, k30-SBC 17 %) tested every
  one of up to 100 000 spots a relation against every part of the board.
- **The swaps** (split72_left 30 %, SNSP-CPU-01 23 %): 42 to 46 % of them
  (the throwaway build's finer split) the crossings before a swap, the
  moved nets' trees built again at the poses the state keeps.
- **The escape measurements** (the global route, `pcb-global`): up to
  11 % (MegaDrive); not changed (a change of the global router).
- **Global placement**: 17 % on the 0xCB panel (30 movable parts, its
  spectral solve), below 3 % elsewhere; not changed (the solve's order
  of operations is its result).
- **Outside the phases** (lowering, set-up, the work-rate calibration):
  0.1 to 1.6 %.
- **The race's probes, the first route and the moves** are the router's
  (the router rounds above); the race's ranking (lowering, RUDY, the
  crossings' total) is milliseconds a placement.

### The changes (bit-identical)

Each its own commit, each the same computation: the same values in the
same order, and the same work booked (the placer's budgets are work, so a
change that booked a unit more or less would move every stop and every
placement after it). Where a cache or a shortcut is new, a unit test
compares it with the direct computation, and the release binary checks it
against the direct one at every use with `PCB_PLACER_CHECK_OVERLAP=1`
(debug builds always do).

1. **The grids' cells without libm's `floor`** (buckets.rs `cell_index`,
   crossings.rs `column`, `row`). x86-64's baseline has no rounding
   instruction, so `floor` is a call (1 to 2.6 % of a placement's
   instructions); a cast truncates towards zero and takes negatives and
   NaN to 0, which is `floor().max(0)` for every input (test: 200 000
   values, the extremes, NaN and the infinities). The bucket query keeps
   its output's length in a register for a cell (the vector's own
   `spare_capacity_mut` and `set_len` per item were a third of its
   instructions on MegaDrive): the same items in the same order.
2. **A linked pair by binary search** (constraints.rs `linked`). Each
   part's partners list is ascending by construction (`link_pairs` walks
   the set in order); the lookup scanned it (a supply pin's part has
   dozens). Test: every pair against the set.
3. **The anneal's wirelength before a move from cached net terms**
   (anneal.rs `lengths`, `cached_wirelength`, `wirelength_terms`,
   `keep_lengths`). A move's wirelength before it is the sum of its nets'
   terms at the poses of now; they are kept per net and replaced by the
   terms measured after a move that is kept, and computed again whenever
   the field (the guidance) changes or the bus groups move parts. The same
   terms summed in the same order: the same bits. Checked against the
   direct sum in debug builds and with `PCB_PLACER_CHECK_OVERLAP`.
4. **The refinement's candidates fold their nets' other pins once**
   (legal.rs `MovingNets`). Among one part's candidate spots and turns
   only that part moves; each of its nets' box of the other pins is folded
   once and a candidate folds only its own pins into it (a power net's
   hundreds of pins were turned again at every candidate). A minimum of
   numbers that are neither NaN nor zero is the same in any order; a net
   with a coordinate of either zero is folded directly. Test: random
   boards and candidates, the bits and the work against `net_cost`.
5. **The escape term from boxes and pads it already has** (escape.rs
   `channel_of_boxes`, `Targets`, `EscapeState::prepare`). A channel took
   both parts' bodies turned again for every neighbour; the grid's parts
   keep theirs since they were placed, the candidate's is turned once. A
   refinement candidate's exits look for each pin's nearest pad of its
   net on another part: those pads stay where they are among the part's
   candidates and are taken once (`prepare`, dropped at any change of the
   state). The same exits, the same overflow; checked against the direct
   exits in debug builds and with `PCB_PLACER_CHECK_OVERLAP`.
6. **Relation penalties from each part's own relations** (constraints.rs
   `PartRelations`). The anneal's and the refinement's penalty of one part
   scanned every relation and every anchor of a near-any relation for the
   part (Glasgow: 82 relations, 23 anchors to each decoupling capacitor);
   the relations it is in are listed once per anneal and refinement, the
   same ones summed in the same order. Test: random relations of all four
   kinds, the bits and the work against `relation_penalty`.
7. **The ring searches visit only the ring** (legal.rs `ring_steps`). The
   legaliser's, the refinement's and the relation repair's spot searches
   scanned every step of the square of side `2 r + 1` for the ring's
   steps; a part without a spot scanned every ring of the board (most of
   video's legalisation). The same steps in the same order. Test: every
   ring and window against the scan.
8. **The relation repair's legality from the parts that may come near**
   (legal.rs `Reach`, `Sweep`). The repair searches up to `(2 r + 1)^2`
   spots around a relation's target and tested each against every part
   of the board (`is_legal(.., 0..count)`, the board's order, the first
   conflict ending the test). Per relation the parts that may come near
   any of its spots are found once (body centres against the parts'
   reach: the farthest corner of anything a part tests legality with, by
   1.5 for turned boxes, plus the widest gap); a part farther away takes
   every sub-test's far path, which keeps it clear and books work that
   depends on the two parts alone (measured once: `clear_of_one` with the
   repaired part far off), so its work is added in the board's order
   without the test. Cutouts are always tested. The same answers and the
   same work; checked against the direct test in debug builds and with
   `PCB_PLACER_CHECK_OVERLAP`; test: random boards with far sides,
   hollow parts, rings and courtyards on the far side, at both legality
   levels.
9. **The anneal's overlap sums from cached boxes** (anneal.rs `PartBoxes`,
   `Blocking`). A pair's overlap built its parts' box lists (far side,
   pads, hollow blocking boxes, courtyards) and freed them again for every
   pair: on DSP-ADAU1452 (354 hollow parts) the lists and their
   allocations were half the overlap sums. Each part's boxes at its pose
   are kept (keyed by the pose's bits) and turned again when it moves; the
   same boxes in the same order and the same work.
10. **The crossings at the poses kept from the kept lines** (crossings.rs
    `involving_kept`). The refinement's starting count of a part and the
    count before each swap built the moved nets' trees again (Prim's
    tree of every pin) at the poses the state keeps them at, inserted them
    into the line grid, counted and removed them: 42 to 46 % of the swaps'
    time on SNSP-CPU-01 and split72_left. Where every pin of those nets is
    where it was kept, the trees are the kept ones, so their crossings are
    the kept lines' count; the work the direct way books (the trees, the
    lines' insertion, the count with each moved net's line met twice, the
    removal's scan of each cell's list, whose order `swap_remove` decides)
    follows from the kept lines and the grid's lists alone. The same count
    and the same work; test: 600 random moves against `involving`.
11. **The legality tests without a list a pair** (legal.rs `turned`,
    `with_rects`). Hollow parts, far sides, rings, pads near a cutout:
    each pair turned its boxes into a new list. They are turned one by one
    now, and a list a test needs twice is held in a buffer of the thread:
    the same answers in the same order, the same work.

### Measured (2026-10-28)

x1871 (main e54c6ada) against the head (3daff389, `pcb-maker-prof4-final`),
pinned on SMT siblings as the router rounds were (one binary on two CPUs
of one core, the other on their siblings 16 higher; the other orientation
with the binaries' CPUs swapped), two threads each, pinned rates,
`PCB_ROUTER_NO_WALL_GUARD=1 PCB_ROUTER_FINGERPRINT=1 PCB_MAKER_NO_CACHE=1`,
each pair inside one reservation of the memory ledger, the machine's load
19 to 38 (the nightly from 02:00 and other agents). The first pairs ran
both orientations at once on two cores; once the nightly held the ledger
(the four-binary reservations waited up to 80 minutes) the rest ran one
orientation a reservation, the second after the first on the same cores
(a pair's two orientations then differ more: 4in1-mini 807 and 649 s for
the same binary). CPU seconds are the process's own threads (utime and
stime from /proc, polled; kicad-cli left out); the change is the geometric
mean of the two orientations' ratios; peak RSS is the process's VmHWM.

**The placement** (`place-kicad-board` of the task, the race's 16 seeds and
the layout's 600 s of work; the eleven dense boards and the four
fingerprint boards). Cells: x1871's two orientations, the head's, the
change, peak RSS (MB), identical placed board in all four runs:

| board | x1871 CPU s | head CPU s | CPU | x1871 MB | head MB | identical |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| 0xCB panel | 224 / 213 | 199 / 187 | -11.7 % | 152 / 146 | 147 / 147 | yes |
| protoesp | 949 / 923 | 874 / 872 | -6.7 % | 67 / 67 | 68 / 68 | yes |
| Glasgow | 1703 / 1703 | 1181 / 1186 | -30.5 % | 96 / 96 | 97 / 97 | yes |
| video | 1158 / 1145 | 989 / 970 | -15.0 % | 82 / 81 | 82 / 82 | yes |
| 4in1-mini | 807 / 649 | 651 / 550 | -17.3 % | 74 / 85 | 86 / 74 | yes |
| 4in1 | 1603 / 1554 | 1264 / 1208 | -21.7 % | 75 / 75 | 75 / 75 | yes |
| jetson | 547 / 478 | 431 / 410 | -17.8 % | 936 / 1110 | 1110 / 937 | yes |
| Sisu | 592 / 674 | 423 / 478 | -28.8 % | 97 / 84 | 85 / 84 | yes |
| SmartSpin2k | 961 / 991 | 543 / 541 | -44.4 % | 278 / 278 | 275 / 276 | yes |
| DSP-ADAU1452 | 667 / 676 | 454 / 464 | -31.7 % | 63 / 72 | 64 / 73 | yes |
| MegaDrive | 839 / 841 | 710 / 727 | -14.4 % | 82 / 82 | 82 / 81 | yes |
| SNSP-CPU-01 | 642 / 608 | 534 / 540 | -14.1 % | 106 / 106 | 106 / 124 | yes |
| split72_left | 679 / 701 | 470 / 473 | -31.7 % | 531 / 531 | 531 / 531 | yes |
| k30-SBC | 223 / 215 | 171 / 175 | -21.1 % | 51 / 50 | 55 / 57 | yes |
| Castor | 237 / 200 | 191 / 164 | -18.6 % | 111 / 110 | 111 / 111 | yes |

The placement's CPU falls 7 to 44 % on every board, 22.6 % as the
geometric mean of the eleven dense boards and 21.7 % of the four
fingerprint boards (22.3 % over the fifteen). Most where the profile's
largest parts were changed: SmartSpin2k (the relation repair), DSP,
Glasgow and split72_left (the overlap boxes, the relation penalties, the
swaps' crossings); least where the legaliser's neighbour queries and
tests dominate (protoesp -7 %, the 0xCB panel -12 %: the legality tests'
lists went, their queries stay) and where the anneal's overlap queries,
crossings term and the escape measurements are most of it (MegaDrive,
SNSP-CPU-01 -14 %). The peak resident size does not change (the caches
are a part's boxes and a net's term: kilobytes).

**The layout** (`layout-kicad-board`, the task budget of 1500 s of work,
six dense boards): the layout's CPU, the placement's (the process's CPU
from `layout: placing` to `layout: placed`), the layout's peak RSS, and
the placed board, the result board and every fingerprint line identical
in all four runs:

| board | x1871 CPU s | head CPU s | CPU | x1871 placement CPU s | head | placement | x1871 MB | head MB | identical |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Glasgow | 2668 / 2625 | 2294 / 2331 | -12.6 % | 1577 / 1538 | 1066 / 1090 | -30.8 % | 464 / 467 | 468 / 468 | yes (placed and result boards, 18 fingerprint lines) |
| SmartSpin2k | 2956 / 3093 | 2558 / 2691 | -13.2 % | 980 / 1004 | 477 / 503 | -50.6 % | 1057 / 1046 | 1065 / 1048 | yes (placed and result boards, 16 fingerprint lines) |
| DSP-ADAU1452 | 3193 / 3505 | 3016 / 3326 | -5.3 % | 702 / 675 | 469 / 441 | -33.9 % | 1222 / 1204 | 1195 / 1226 | yes (placed and result boards, 46 fingerprint lines) |
| MegaDrive | 2960 / 3363 | 2834 / 3146 | -5.4 % | 888 / 886 | 716 / 724 | -18.8 % | 1752 / 1750 | 1771 / 1760 | yes (placed and result boards, 20 fingerprint lines) |
| SNSP-CPU-01 | 3229 / 2803 | 2937 / 2773 | -5.2 % | 731 / 700 | 583 / 586 | -18.3 % | 1652 / 1651 | 1669 / 1659 | yes (placed and result boards, 20 fingerprint lines) |
| split72_left | 4669 / 3593 | 4474 / 3517 | -3.2 % | 643 / 585 | 433 / 407 | -31.5 % | 2676 / 2695 | 2609 / 2631 | yes (placed and result boards, 62 fingerprint lines) |

The layout's placement falls 18 to 51 % (31.6 % as the geometric mean);
the layout as a whole falls 3 to 13 % (7.6 % as the geometric mean of
the six):
the placement is 14 to 59 % of a layout's CPU (Glasgow's placement takes
its whole share; split72_left's routing is three quarters of its CPU),
and the rest (the race's probes, the routes, the moves) is the router's,
which this round did not touch.

**Identical.** Every placement of the pairs above (eleven dense and four
fingerprint boards, all four runs of each). Each commit on its own:
the binary of each of the eleven commits against x1871, placements at
30 s of work (`PCB_PLACER_WORK_SECONDS=30`) of SmartSpin2k, DSP-ADAU1452,
4in1-mini, MegaDrive, Glasgow and split72_left: the same board from all
twelve binaries on each. The head against x1871 at 60 s of work on all
fifteen boards: the same placements; in check mode
(`PCB_PLACER_CHECK_OVERLAP=1`, every cache and the sweep against the
direct computation at every use) on split72_left, Glasgow and
DSP-ADAU1452 at 80 s and on k30-SBC, SmartSpin2k, 4in1-mini and DSP at
120 s: no difference. The layouts: the six pairs above, and single
layouts of the other five dense boards (0xCB panel, protoesp, video,
4in1-mini, 4in1; x1871 and the head, unpinned): placed board, result
board and fingerprint lines identical. The fingerprints (route mode,
Castor, k30-SBC, jetson and Sisu from build/github-layout-v16, a quarter
budget, two threads): every fingerprint line and the routed boards
identical (Castor's still dad3a6cb3787). After the rebase on main
5905bf9a (x1872): the rebased head against x1872 at 60 s of work on all
fifteen boards, the same placements. `scripts/check.sh --fast` passes.

**The work clock.** Placement budgets are work; the work booked is the
same by construction (each change books what the computation it replaces
booked), so a faster placement does the same work, and the measured
placer rate (the calibration's canned anneal) is the anneal's: it does not
move with these changes by more than the anneal's own share
(`calibrate-work-rate` was not rerun).

### Candidates not taken

- **The anneal's overlap "before" cached per part**: a move's overlap
  before it is the sum over the parts the bucket query finds, in the
  query's order; the query's order follows the cells' lists, which every
  move attempt permutes (`swap_remove` on the way out, a push on the way
  back), so a cached sum can differ in its last bits from the sum the
  anneal takes. Not exact; not taken.
- **The bucket query from a coarser index** (MegaDrive's overlap sums and
  legality tests: a reach of the largest far-side part's makes every query
  span hundreds of cells): the order of the items (their first cell in the
  query's row-major scan, then their place in that cell's list) can be
  reproduced from each item's cell range, but the count of distinct items
  the work books needs the scan or a 2-D count structure; at the boards'
  sizes no clear gain over the scan. Not tried.
- **The anneal's crossings term** (2 to 19 % since e54c6ada): each move's
  `before` counts the moved nets' kept lines through the grid; a cached
  count would have to book the same work, which depends on every line
  in the cells the moved nets' lines pass. Not taken.
- **The legaliser's and the refinement's neighbour queries** (the rest of
  protoesp's 59 % and the 0xCB panel's 35 % of legality): the query's
  work (cells and items) is booked, so a cheaper query must still count
  the items it skipped; the sweep's reasoning (far parts' work from their
  far path) would serve, with the bucket query's own order of the near
  parts reproduced. The next step.
- **The escape measurements** (the global route, `pcb-global`, up to 11 %
  on MegaDrive) and **global placement** (17 % on the 0xCB panel): the
  global router's and the spectral solve's own rounds.

**A debug switch that booked work**: `PCB_PLACER_DEBUG`'s refinement line
counted the board's crossings again (`resync`), booking that count's work,
so a run with the switch could place otherwise than one without (the
seeds' work decides the race's batches); its work is taken back now
(37b81380, after the measured head; the path runs only with the switch:
the placements without it are the head's).
The attribution build's own placements differ from x1871's on some boards
(SmartSpin2k, DSP-ADAU1452, MegaDrive: 3 to 16 % less work): its timing
of an anneal swap's two relation costs added them to each other before
adding them to the wirelength, another rounding of the same sum; its
categories time the same code paths, so the shares stand as estimates.
`place-kicad-board` gave the layout's placement on every board measured
(the same work and seeds as the layout pairs' and the single layouts'
placements of x1871).

## The third placer round (2026-10-28)

The legaliser's and the refinement's legality after the second round,
whose "candidates not taken" named their neighbour queries the next
step. Branch placer-profile-3: sixteen commits on main f26f822f (x1873,
the second round), rebased on main 39fc92d6 (x1874: the sub-boards' tabs
and frames, no placer code) as 7f4b1082 to 19c845aa, the measured head
(`build/bin/pcb-maker-prof5-final`), and a seventeenth, 297f458f, that
takes the fifteenth back (`-final2`, below). The branch was rebased on
later mains after the measurements (the router's via-in-pad and
fan-out, the fast-protocol harness, the ledger's drain: no placer code;
the rebased head's placements checked against x1875 at 30 s of work on
six boards), so its commits' hashes are newer than these. Each commit's
binary before
the rebase is `build/bin/pcb-maker-prof5-c1b` to `-c13` (`-c8`: the
seventh and eighth), after it `-r1` (the fourteenth), `-c15`, `-c16`;
outputs in `build/prof5-*`.

### How

perf could not be fetched in the sandbox again; the placement was
attributed as in the second round, with finer categories:

- **Categories**: a throwaway build (branch placer-profile-3-attr, not
  for merging: `attr.rs`, a time-stamp counter per category,
  `PCB_PLACER_ATTR`) times the legaliser's bucket query, legality test,
  escape term, crossings and near relations, the eviction's queries and
  tests, the refinement's query, legality test, wirelength and
  relations, escape term, escape measurements, crossings and swaps, and
  the relation repair's legality; each phase's CPU (`PCB_PLACER_DEBUG`)
  split by its categories' ticks; `place-kicad-board` at the layout's
  budget, two threads, pinned rates, one run a board (shares, not CPU
  changes).
- **Counts** (`PCB_PLACER_ATTR_COUNT`, runs of their own: they slow the
  tests): per caller the spots tested, the parts each bucket query found
  and the cells it looked at, the pair tests (`clear_of_one`), and how
  many of the pairs lay apart by the boxes of `Apart` (below).
- **Callgrind** on short placements (`PCB_PLACER_WORK_SECONDS` 10 or 15,
  one thread) with the collection toggled on one phase
  (`--collect-atstart=no --toggle-collect='*legalize_keeping*'`, or
  `'*refine_with*'`): the profiling build of "How to profile" for the
  functions inside; the release binaries (they keep their symbols) for
  each commit's instructions, which do not depend on the machine's load.

CPU pairs, identity checks and fingerprints as the second round's
("Measured"), against x1874; on the shared machine every
`place-kicad-board` reserves a KiCad check (2253 MB) in the memory
ledger, so the later pairs ran one orientation a reservation
(`--halves`, 2500 MB) and the callgrind runs two at a time.

### Where the placement's CPU goes (x1873, f26f822f)

Shares of the placement's CPU (the timing build: its counter reads add a
little to the timed categories):

| category (% of the placement's CPU) | 0xCB panel | protoesp | Glasgow | video | 4in1 | Castor | k30-SBC | jetson | Sisu |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| placement CPU s (2 threads, the timing build) | 188 | 810 | 1015 | 990 | 1063 | 182 | 166 | 481 | 450 |
| global placement | 21.0 | 3.2 | 0.5 | 1.0 | 0.3 | 3.8 | 2.2 | 0.6 | 0.8 |
| anneal | 15.0 | 11.7 | 28.7 | 8.6 | 23.1 | 67.6 | 60.2 | 46.6 | 73.0 |
| legaliser: bucket query | 3.3 | 15.6 | 2.7 | 11.3 | 6.5 | 0.7 | 0.6 | 1.0 | 0.1 |
| legaliser: legality test | 38.5 | 64.1 | 11.7 | 11.9 | 39.2 | 1.2 | 0.6 | 10.5 | 0.4 |
| legaliser: crossings | 0.5 | 0.2 | 17.4 | 61.8 | 3.1 | 0.8 | 5.7 | 0.6 | 0.4 |
| legaliser: escape term | 0.0 | 0.0 | 1.1 | 1.2 | 0.5 | 0.1 | 0.1 | 0.1 | 0.0 |
| legaliser: eviction (queries, tests) | 10.4 | 0.4 | 0.0 | 1.0 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 |
| legaliser: the rest (ring scan, near relations) | 0.7 | 1.7 | 0.4 | 1.5 | 1.4 | 0.1 | 0.2 | 0.1 | 0.0 |
| refinement: bucket query | 0.7 | 0.4 | 1.7 | 0.0 | 1.5 | 1.4 | 1.1 | 1.6 | 1.6 |
| refinement: legality test | 5.0 | 1.3 | 9.6 | 0.0 | 6.9 | 2.6 | 1.0 | 13.2 | 5.8 |
| refinement: wirelength and relations | 0.2 | 0.1 | 9.2 | 0.1 | 0.9 | 1.5 | 5.3 | 0.9 | 1.3 |
| refinement: escape term | 0.0 | 0.0 | 6.5 | 0.1 | 8.5 | 5.5 | 5.0 | 5.5 | 5.3 |
| refinement: escape measurements | 0.0 | 0.0 | 0.2 | 0.6 | 0.0 | 2.6 | 2.4 | 3.3 | 1.9 |
| refinement: crossings | 0.5 | 0.1 | 9.1 | 0.1 | 0.6 | 0.8 | 1.0 | 1.9 | 0.9 |
| refinement: equal-part swaps | 0.2 | 0.4 | 0.3 | 0.6 | 0.4 | 7.3 | 1.6 | 10.8 | 3.6 |
| relation repair: legality | 1.7 | 0.4 | 0.3 | 0.0 | 6.3 | 2.6 | 11.0 | 2.2 | 4.0 |
| refinement and repair: the rest | 0.1 | 0.1 | 0.4 | -0.2 | 0.4 | 0.3 | 1.2 | -0.1 | 0.6 |
| outside the phases | 2.1 | 0.3 | 0.2 | 0.2 | 0.2 | 1.2 | 0.7 | 1.2 | 0.5 |

- **The legaliser's legality is pair test after pair test** at spots that
  are almost all illegal: protoesp's legaliser tested 442 million spots
  at the layout's budget (0.03 % legal: a part without room scans the
  board ring by ring, at every pass of `seat_on_marks` and
  `legalize_stuck_first`), 4.8 pair tests a spot, the bucket query
  finding 10.4 parts in 32 cells. 86 % of those pairs lay farther apart
  than any gap the test asks for, by the boxes around the two parts (the
  0xCB panel 63 %, Glasgow 72 %, video 53 %, 4in1 30 %); of the
  refinement's 78 % on protoesp and Glasgow, 52 % on the panel, 40 % on
  4in1; of the relation repair's sweep's near parts 91 to 95 %. The
  second round's sweep reach (`Reach`, a radius by 1.5) judged almost none
  of the legaliser's pairs far.
- **Inside the legaliser** (callgrind at 10 to 15 s of work): two thirds
  of the 0xCB panel's is the cutouts' polygon test (`rect_meets_polygon`:
  the panel's slots run around whole copies, thousands of points, and
  every pad of a part near one walked every edge); protoesp's the bucket
  query (27 %), the pair tests' hole rings (13 %), hollow boxes (12 %),
  bodies (8 %) and the binary searches of `lies_on` and `bare` (10 %);
  4in1's the hollow boxes' lists (30 %), the fit test's walk of the board
  outline (`inside_outline`, 22 %, every legal spot), the query (10 %),
  the crossings (8 %).
- **video's legaliser is its crossings** (62 % of its placement: the
  second round's "rest" was not its ring scan): every legal spot within
  `CROSSING_REACH` of the closest builds the part's nets' trees, inserts
  their lines into the line grid, counts their crossings and takes them
  out again; 60 % of those instructions the count (the crossing tests
  themselves), 18 % the insertion and removal, 9 % the trees. Glasgow's
  crossings: 17 % (legaliser) and 9 % (refinement).
- **Not this round's**: the anneal (9 to 73 %; two thirds of the four
  fingerprint boards), global placement (21 % on the panel), the
  refinement's escape term and swaps (5 to 11 %).

### The changes (bit-identical)

Each its own commit, each the same computation: the same answers, the
same work booked; a unit test against the direct computation each (with
mutation checks: the tests fail with a box kind left out, a smaller
gap, a band too few, the courtyards or the nets dropped), and checked
against it at every use in debug builds and with
`PCB_PLACER_CHECK_OVERLAP=1`. The instructions of the legaliser
(callgrind, `legalize_keeping`, 10 s of work, one thread, release
binaries), each commit against the one before:

| commit | protoesp | video | 4in1 |
| --- | ---: | ---: | ---: |
| x1873 (f26f822f) | 52.2 G | | |
| 1. bucket answers | 40.7 G (-22 %) | | |
| 2. pairs apart by boxes | 17.7 G (-56 %) | | |
| 3. cutouts' outlines by bands | 14.0 G (-21 %) | 84.4 G | |
| 4. fresh trees beside the grid | | 61.7 G (-27 %) | |
| 4 and 5 (the sweep: the repair, not here) | 13.8 G (-1.4 %) | | |
| 6. the fit test's outline walk | 13.7 G (-0.6 %) | | |
| 7 to 9 (escape grid, memory, flags) | 12.8 G (-6.8 %) | | |
| 10. full polygon test by bands | 12.6 G (-1.6 %) | | |
| 11. linked work once | 11.8 G (-6.2 %) | | 28.3 G |
| 12. boxes turned once a pose | 11.9 G (+0.7 %) | 60.1 G | 23.9 G (-16 %) |
| 13 to 15 (Prim's lists, check mode, forgetting by cells) | 12.4 G (+4.6 %) | 59.8 G (-0.5 %) | |
| 16. the searched part's frame | 12.4 G (-0.4 %) | | |
| 17. forgetting every answer again | 12.2 G (-1.4 %) | | |

The legaliser of the 0xCB panel (15 s of work, the profiling build):
131.2 G instructions on x1873, 24.6 G after the seventh commit, 22.1 G
after the thirteenth; 4in1's 51.9, 32.0, 25.3 G; video's 106.5, 71.6,
69.6 G; protoesp's 54.3, 13.8, 12.0 G (10 s). 4in1-mini's refinement
(`refine_with`, 15 s): 6.55, 5.46, 5.46 G.

1. **The bucket queries keep their answers** (buckets.rs `Answers`,
   `Grid::with_answers`; the legaliser's, the refinement's and the
   eviction's grids). The same cells hold the same parts in the same
   order until a part goes in or out, so a query of the same cells has
   the same answer: kept by the cells' range in a direct-mapped table of
   4096 slots, forgotten at an insertion or removal. A spot search steps
   0.25 to 0.5 mm in cells of 2.5 mm (video's spots looked at 60 cells
   for 1.7 parts). The work booked is the query's, as before. Test:
   random grids, items going in and out, queries on a lattice, against a
   grid without kept answers.
2. **Pairs apart by their boxes take the test's far path untested**
   (legal.rs `Apart`; the legaliser, the refinement's candidates, the
   eviction). Per part a box in its own frame around everything
   `clear_of_one` compares beside its body (pads, far side and far
   copper, far courtyards, hollow boxes, courtyards, hole rings), on the
   board at its pose together with its body (a disc's radius on both
   axes); the gap: the largest a test asks for (the spacing, the copper
   clearance, the mask gap, two copper shares) and 10^-6 mm. Two parts
   at quarter turns whose boxes lie farther apart than that on one axis
   are apart on that axis in every comparison of the test (every
   rectangle there is axis-aligned, a disc within its box), which takes
   its far path: clear, booking work of the two parts alone (the second
   round's sweep argument), measured once a pair with the part 10^7 mm
   off. A part turned off a quarter turn (separating axes on its own
   axes) and a cutout are always tested. Test: random boards of every
   kind of part at both legality levels, with shared halos, copper
   shares, marks, linked pairs, pins of three nets and slots of many
   points, against `is_legal`.
3. **The cutouts' polygon tests from their outline's edges by bands**
   (legal.rs `Outline`). A cutout of 32 edges or more keeps its box and
   its edges by bands of y (an edge in every band its y range touches):
   the first pass of `rect_meets_polygon` (is an edge near the pad's box;
   else the even-odd side of its first corner) takes the edges of the
   bands of the box's y range and of the corner's y (a band function
   monotonic in y: an edge near the box, or crossing the corner's ray,
   lies in one of them). The work stays booked by the outline's length.
   Test: ragged rings, rings on a lattice (edges along the bands'
   borders), plain and turned boxes, against the pass over every edge.
4. **The crossings of fresh trees without their lines in the grid**
   (crossings.rs `involving`). `involving` inserted a spot's or a
   candidate's trees into the line grid, counted them against it and took
   them out again. The fresh lines meet the kept ones through the grid as
   before, each other where their walks share a cell (a line is in every
   cell it passes: two fresh lines met in the grid exactly when they
   share one), each crossing of two once; the work `involving_direct`
   books (the trees, the insertion's cells, the count's cells and lines
   met, the fresh ones among them, the removal's cells and scans, a fresh
   line found behind its cell's kept lines where `swap_remove` leaves it,
   `tail_place`) is booked from those. The ids go back to the free list as
   `drop_lines` gave them. Test: the second round's exactness test with
   `involving` against `involving_direct` (count and work) at every move,
   four board sizes; a mutation of the removal's scan fails it.
5. **The relation repair's sweep with the boxes** (`Sweep::is_legal`): a
   near part apart from the spot by the boxes takes its far work from the
   sweep's prefix untested (91 to 95 % of the sweep's tests). Test: the
   sweep's test on the random boards.
6. **The fit test's outline walk** (`inside_outline`): the corners'
   even-odd tests from the board outline's bands (an outline of 12 edges
   or more), and per outline edge its four corner orientations first:
   with all four on one side of the edge's line no side crosses it
   (`segments_cross`'s second condition, the same orientations), so the
   sides' are not taken (sixteen orientations an edge before, four now).
   Test: rounded, ragged and lattice outlines against the walk before.
7. **The escape term's grid keeps its answers** (escape.rs,
   `Grid::with_answers`).
8. **The kept answers' memory bounded**: an answer whose slot another took
   is kept anew, so the items grew with the misses; at 2^20 items a grid
   grew to eight megabytes (protoesp's peak 67 to 76 MB in the first
   pairs), at most 2^16 now.
9. **Marks and obstacles from per-part flags** (legal.rs `Facts`):
   `Constraints::lies_on` and `halo_free` were binary searches of their
   lists, two to four a pair test; flags now, the lists' own answers where
   one is not ascending. Test: every pair of the random boards.
10. **The cutouts' full polygon test by bands too**
    (`rect_meets_outline`): a pad near a slot's edge took nine passes over
    every edge; the corners' sides from their bands, the polygon's corners
    inside the box from the bands of its y range (a hair wider for the
    rounding), an edge's sides only where the box's corners lie on both
    sides of its line. Test: against the test before, a thousand pads of
    each answer near the edges.
11. **The linked pairs' lookup work once a search** (`Apart::linked_work`):
    a logarithm and a rounding, libm's calls, at every spot.
12. **The pair tests from boxes turned once a pose** (legal.rs `Turned`):
    hollow boxes, courtyards, far side, far copper, far courtyards, hole
    rings and pads of a part turned to its pose and kept while it stays
    there, its pads' nets once; the other parts of a search stay put, the
    searched part is at one spot for all its tests there. The same boxes
    (the same `turned`). Test: the random boards with hollow parts of
    split courtyards against parts with holes.
13. **Prim's trees without lists** (`tree_into`, `spanning_tree_into`):
    a list a net and Prim's two for every spot.
14. **Check mode compares the tested pairs too**: with
    `PCB_PLACER_CHECK_OVERLAP` every pair test from the kept facts against
    the direct one.
15. **An item going in or out forgot only the answers of its cells**
    (`Answers::forget`): did not pay (see below), taken back by the
    seventeenth commit.
16. **The searched part's body and box from its frame** (`Apart::spot`,
    `Frame`, `turned_offset`): its body's offset and rectangle and its
    box's turned offset and half size kept per part and angle, a spot
    adds its position: the same sums as `rect` and `turned` (test: bit for
    bit at quarter, 30 and 45 degree turns, positions off the lattice).
17. **Forgetting every answer again** (297f458f, reverts the fifteenth):
    its list of live slots, scanned at every insertion and removal, grew
    to 2^16 entries between clears.

### Measured (2026-10-28)

**The placement** (`place-kicad-board` of the task, the race's 16 seeds
and the layout's 600 s of work; pinned pairs on SMT siblings, two
threads, pinned rates, `PCB_ROUTER_NO_WALL_GUARD=1
PCB_ROUTER_FINGERPRINT=1 PCB_MAKER_NO_CACHE=1`; the five boards above
both orientations at once, the other ten one orientation a reservation
on the same cores, the machine's load 14 to 40). x1874 against the
measured head (19c845aa): CPU seconds of both orientations, the change
(the geometric mean of the orientations' ratios), peak RSS (MB), the
placed board identical in all four runs:

| board | x1874 CPU s | head CPU s | CPU | x1874 MB | head MB | identical |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| 0xCB panel | 198 / 195 | 109 / 112 | -43.8 % | 146 / 146 | 147 / 147 | yes |
| protoesp | 782 / 839 | 394 / 392 | -51.5 % | 66 / 66 | 69 / 71 | yes |
| Glasgow | 1006 / 1005 | 732 / 739 | -26.9 % | 95 / 95 | 99 / 95 | yes |
| video | 986 / 967 | 693 / 696 | -28.9 % | 80 / 80 | 81 / 80 | yes |
| 4in1-mini | 510 / 524 | 359 / 375 | -29.1 % | 74 / 73 | 85 / 74 | yes |
| 4in1 | 1281 / 1212 | 816 / 815 | -34.6 % | 75 / 74 | 75 / 76 | yes |
| SmartSpin2k | 414 / 366 | 269 / 239 | -35.0 % | 292 / 291 | 291 / 292 | yes |
| DSP-ADAU1452 | 343 / 323 | 307 / 283 | -11.4 % | 63 / 62 | 64 / 65 | yes |
| MegaDrive | 698 / 639 | 446 / 442 | -33.5 % | 81 / 81 | 82 / 81 | yes |
| SNSP-CPU-01 | 558 / 495 | 496 / 457 | -9.3 % | 105 / 105 | 107 / 107 | yes |
| split72_left | 329 / 343 | 313 / 323 | -5.4 % | 530 / 530 | 530 / 530 | yes |
| jetson | 357 / 362 | 311 / 319 | -12.2 % | 936 / 935 | 936 / 936 | yes |
| Sisu | 431 / 386 | 393 / 360 | -7.9 % | 96 / 83 | 86 / 85 | yes |
| k30-SBC | 186 / 182 | 166 / 160 | -11.6 % | 51 / 55 | 53 / 57 | yes |
| Castor | 202 / 202 | 193 / 194 | -4.2 % | 127 / 126 | 111 / 128 | yes |

The placement's CPU falls 4 to 52 %: 29.4 % as the geometric mean of the
eleven dense boards, 9.0 % of the four fingerprint boards, 24.5 % of the
fifteen. Most where the legaliser's tests were most: protoesp -52 %, the
0xCB panel -44 %, SmartSpin2k, 4in1 and MegaDrive -34 to -35 %, 4in1-mini
and video -29 %, Glasgow -27 %; least where the anneal is most of it
(Castor -4 %, split72_left -5 %, Sisu -8 %, SNSP-CPU-01 -9 %). The peak
resident size moves by a few megabytes (the kept answers, the turned
boxes).

**Taking the fifteenth commit back** (297f458f, `-final2`, against the
measured head; pinned pairs, one orientation a reservation):

| board | head CPU s | 297f458f CPU s | CPU | head MB | 297f458f MB | identical |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| protoesp | 355 / 362 | 353 / 357 | -0.9 % | 68 / 70 | 72 / 71 | yes |
| Glasgow | 661 / 590 | 664 / 587 | +0.0 % | 95 / 96 | 96 / 96 | yes |
| 4in1 | 710 / 664 | 695 / 651 | -2.1 % | 75 / 76 | 76 / 76 | yes |
| jetson | 309 / 352 | 304 / 351 | -0.9 % | 935 / 936 | 937 / 1111 | yes |
| Castor | 152 / 154 | 152 / 155 | +0.3 % | 112 / 112 | 112 / 112 | yes |

-0.7 % as the geometric mean of the five: within the pairs' noise (the
instructions say 1.4 % of protoesp's legaliser); the seventeenth commit
is kept for the bounded scan, not for a measured gain.

**The layout** (`layout-kicad-board`, the task's 1500 s of work, six dense
boards, one orientation a reservation): the layout's CPU, the placement's
(the process's CPU from `layout: placing` to `layout: placed`), the
layout's peak RSS, the placed board, the result board and every
fingerprint line identical in all four runs:

| board | x1874 CPU s | head CPU s | CPU | x1874 placement CPU s | head | placement | x1874 MB | head MB | identical |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Glasgow | 2481 / 1847 | 2202 / 1658 | -10.7 % | 1136 / 916 | 782 / 657 | -29.7 % | 464 / 464 | 466 / 466 | yes (placed and result boards, 18 fingerprint lines) |
| SmartSpin2k | 2149 / 2004 | 1930 / 1844 | -9.1 % | 433 / 484 | 229 / 233 | -49.6 % | 1056 / 1074 | 1060 / 1067 | yes (placed and result boards, 12 fingerprint lines) |
| DSP-ADAU1452 | 2868 / 2870 | 2809 / 2816 | -2.0 % | 405 / 411 | 328 / 346 | -17.4 % | 1203 / 1197 | 1202 / 1207 | yes (placed and result boards, 46 fingerprint lines) |
| MegaDrive | 2370 / 2262 | 2189 / 2108 | -7.2 % | 591 / 670 | 389 / 427 | -35.2 % | 1754 / 1775 | 1760 / 1752 | yes (placed and result boards, 20 fingerprint lines) |
| SNSP-CPU-01 | 2819 / 2426 | 2669 / 2349 | -4.2 % | 567 / 585 | 492 / 529 | -11.5 % | 1658 / 1677 | 1660 / 1665 | yes (placed and result boards, 20 fingerprint lines) |
| split72_left | 4023 / 3957 | 3948 / 3997 | -0.4 % | 426 / 438 | 397 / 408 | -6.8 % | 2658 / 2683 | 2677 / 2645 | yes (placed and result boards, 62 fingerprint lines) |

The layout's placement falls 7 to 50 % (26.6 % as the geometric mean of
the six), the layout 0.4 to 11 % (5.7 %): the rest of a layout is the
router's (the race's probes, the routes, the moves), which this round did
not touch. SmartSpin2k's and DSP-ADAU1452's layouts end with exit 1 (the
verdict's) on both binaries alike.

**Identical.** Every placement of the pairs (fifteen boards, all four runs
of each) and of the layout pairs (placed boards, result boards,
fingerprint lines). Each commit on its own: its binary against x1873
(before the rebase) or the rebased fourteenth (after), placements at 30 s
of work of protoesp, the 0xCB panel, video, 4in1, Glasgow and SmartSpin2k:
the same board from all of them. The head against x1874 at 60 s of work
on all fifteen boards: the same placements; and `-final2` too. In check
mode (`PCB_PLACER_CHECK_OVERLAP=1`: every kept answer, box skip, kept
fact, band and fresh tree against the direct computation at every use)
on protoesp, the 0xCB panel, 4in1, video, Glasgow, SmartSpin2k and jetson
at 60 s: no difference, and the placements those of x1874 in check mode
(check mode books the anneal's check, so its placements are not the
plain ones; both binaries alike). The route-mode fingerprints (Castor,
k30-SBC, jetson, Sisu from build/github-layout-v16, a quarter budget, two
threads): every fingerprint line and the routed boards identical
(Castor's still dad3a6cb3787). `scripts/check.sh --fast` passes.

**The work clock**: the work booked is the same by construction (each
change books what the computation it replaces booked).

### Candidates not taken

- **Forgetting only the answers of the cells an item meets** (the
  fifteenth commit, taken back): it kept the answers across the parts of
  a legalisation, but its list of live slots, scanned at every insertion
  and removal, grew to 2^16 entries between clears: protoesp's legaliser
  took 4.6 % more instructions (12.4 G against 11.9 G). A list without
  repeats (at most the 4096 slots) would bound the scan; not tried.
- **A two-dimensional index of the cutouts' edges**: after the bands, the
  0xCB panel's polygon tests are still a third of its legaliser (about 30
  band edges a pad near a slot); a grid of cells would find the edges near
  a pad among fewer.
- **The crossing tests themselves** (video: the count is 60 % of its
  legaliser after the fourth commit): each spot's fresh lines against the
  kept lines in their cells, booked per line met; the lines a spot meets
  change with every spot.
- **The anneal** (Castor, k30-SBC, Sisu, jetson, split72_left, SNSP: 46 to
  73 % of their placement): the box prefilter could serve its overlap sums
  (a pair apart adds 0.0 and books its work from counts), not tried this
  round; the second round's notes on its crossings term and its cached
  sums stand.
- **The refinement's swaps' and the alignment's legality** (`is_legal`
  against every part, 96 to 98 % of the pairs apart by boxes): below 0.2 %
  of a placement; not changed.

## The fourth placer round (2026-11-04)

The anneal after the third round, whose "candidates not taken" named it
(two thirds of the fingerprint boards' placement, its crossings term on
by default since 2026-10-28), the 0xCB panel's slot edges, video's
crossing tests and the race's ripped count (each probe's router copied
and ripped up since 2026-11-03). Branch placer-profile-4: fourteen commits, the measured head
`build/bin/pcb-maker-prof6-r13` (thirteen commits on main 6a95300a, x1885,
the pairs' off side, whose placements are x1884's), the fourteenth (it
takes the third back) measured on its own as `-r14`, the branch rebased
on main 67ad67a2 (x1888) as `-final`; the commits' binaries on their way
`-c1` to `-c6` (on 84ba6893, x1881), `-r1b`, `-r9` to `-r12` (on
9d6844aa, x1884); outputs in `build/prof6/`.

### How

As in the third round (perf could not be installed):

- **Categories**: a throwaway build (branch placer-profile-4-attr, not
  for merging: `attr.rs`, a time-stamp counter per category and thread,
  printed with each `PCB_PLACER_DEBUG` phase line under
  `PCB_PLACER_ATTR`) times the anneal's wirelength terms, relations,
  overlap sums and the crossings term's three steps (`before`, `after`,
  `settle`), the legality tests, the cutouts' polygon tests (inside the
  tests), the bucket queries, the escape term and its measurements and the
  crossings (`involving`, `involving_kept`, `kept`) by phase; each phase's
  thread CPU split by its categories' ticks; `place-kicad-board` at the
  layout's budget on x1881's code (main 84ba6893), two threads, pinned
  rates, one run a board (shares, not CPU changes). The same build
  counted the new lines that are a kept line again, bit for bit (the
  anneal's `after` and the legaliser's `involving`).
- **Callgrind** of the release binaries (they keep their symbols) on
  short placements (`PCB_PLACER_WORK_SECONDS=10`, one thread) with the
  collection on one phase (`--toggle-collect='*anneal*anneal*'` or
  `'*legalize_keeping*'`): MegaDrive's and DSP-ADAU1452's anneal, video's
  legaliser, each change against the binary before it (the same work, so
  the same steps: instructions compare).
- **CPU pairs** as the rounds before (pinned on SMT siblings, one binary
  on two CPUs, the other on their siblings, then swapped; two threads,
  pinned rates, `PCB_ROUTER_NO_WALL_GUARD=1 PCB_ROUTER_FINGERPRINT=1
  PCB_MAKER_NO_CACHE=1`; the process's own CPU from /proc, read at its
  exit before it is reaped, kicad-cli left out; at most three
  `place-kicad-board` in flight, each reserving KiCad's check in the
  memory ledger): `build/prof6/pairs.py`, `summary.py`.

### Where the placement's CPU goes (x1881's code, 84ba6893)

Shares of the placement's CPU (the timing build, `place-kicad-board` at
the layout's budget; `build/prof6/profile`, `build/prof6/attr_table.py`;
the "anneal: crossings" rows are the term's three steps, the legaliser's
"crossings" `involving`'s fresh trees and `kept`):

| category (% of the placement's CPU) | 0xCB panel | protoesp | video | 4in1-mini | glasgow | SmartSpin2k | DSP | 4in1 | MegaDrive | split72_left | Castor | SNSP-CPU-01 | k30-SBC | jetson | Sisu |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| placement CPU s (2 threads, the timing build) | 66 | 290 | 460 | 265 | 531 | 255 | 278 | 573 | 362 | 386 | 163 | 477 | 150 | 434 | 431 |
| global placement | 35.9 | 7.2 | 1.7 | 0.9 | 0.6 | 5.5 | 0.5 | 0.3 | 0.2 | 0.6 | 3.9 | 0.6 | 2.4 | 0.6 | 0.8 |
| anneal: wirelength | 4.3 | 1.8 | 2.3 | 2.1 | 5.4 | 0.9 | 11.3 | 2.7 | 3.1 | 11.8 | 8.1 | 5.5 | 9.8 | 5.8 | 10.9 |
| anneal: relations | 1.2 | 0.6 | 1.6 | 0.2 | 7.6 | 0.8 | 0.8 | 0.7 | 0.9 | 4.0 | 4.4 | 1.7 | 12.7 | 1.5 | 3.2 |
| anneal: overlap | 13.9 | 13.9 | 2.6 | 21.9 | 11.4 | 2.4 | 34.1 | 18.4 | 38.8 | 10.2 | 25.8 | 29.2 | 12.6 | 16.9 | 24.1 |
| anneal: crossings before | 4.5 | 5.1 | 3.2 | 5.0 | 10.0 | 1.3 | 16.8 | 5.7 | 11.1 | 11.1 | 14.7 | 12.4 | 11.0 | 14.6 | 19.3 |
| anneal: crossings after | 0.9 | 3.1 | 4.7 | 2.8 | 7.4 | 0.7 | 7.9 | 5.0 | 11.6 | 6.5 | 11.5 | 9.9 | 14.8 | 9.1 | 13.5 |
| anneal: crossings settle | 0.1 | 0.1 | 0.7 | 0.4 | 0.5 | 0.1 | 0.6 | 0.5 | 1.4 | 0.2 | 0.6 | 0.8 | 1.8 | 0.3 | 0.8 |
| anneal: the rest | 3.9 | 2.3 | 1.1 | 1.7 | 1.8 | 1.1 | 5.7 | 1.6 | 1.8 | 2.5 | 8.1 | 3.7 | 6.6 | 2.8 | 4.2 |
| legaliser: crossings | 0.6 | 0.4 | 57.3 | 2.7 | 14.2 | 3.2 | 0.3 | 2.8 | 4.4 | 3.3 | 0.5 | 0.6 | 3.9 | 0.4 | 0.2 |
| legaliser: legality (cutouts excl.) | 5.9 | 36.1 | 11.1 | 15.6 | 5.2 | 0.6 | 0.5 | 26.4 | 5.0 | 0.3 | 0.9 | 0.4 | 0.5 | 4.3 | 0.2 |
| legaliser: cutouts' polygon tests | 9.4 | 6.3 | 0.0 | 0.0 | 0.0 | 0.1 | 0.0 | 0.0 | 1.7 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 |
| legaliser: bucket query | 2.3 | 14.8 | 5.9 | 1.3 | 1.1 | 0.1 | 0.1 | 3.6 | 2.7 | 0.1 | 0.4 | 0.2 | 0.3 | 0.5 | 0.0 |
| legaliser: escape term | 0.0 | 0.0 | 1.8 | 0.8 | 1.6 | 0.3 | 0.0 | 0.6 | 0.2 | 0.1 | 0.1 | 0.0 | 0.2 | 0.1 | 0.0 |
| legaliser: the rest | 4.9 | 5.4 | 3.7 | 1.0 | 0.7 | 0.2 | 0.0 | 2.5 | 0.7 | 0.0 | 0.2 | 0.1 | 0.2 | 0.2 | 0.0 |
| refinement: crossings | 0.6 | 0.7 | 0.5 | 2.6 | 6.8 | 6.0 | 3.4 | 0.9 | 3.5 | 16.4 | 3.0 | 14.2 | 1.0 | 2.8 | 1.0 |
| refinement: legality | 2.1 | 0.7 | 0.0 | 20.6 | 3.6 | 3.8 | 2.2 | 5.9 | 1.1 | 1.1 | 1.3 | 1.4 | 0.4 | 2.3 | 1.0 |
| refinement: escape term | 0.0 | 0.0 | 0.2 | 16.0 | 5.8 | 10.0 | 2.3 | 10.6 | 0.6 | 1.2 | 2.7 | 1.5 | 2.1 | 1.7 | 1.9 |
| refinement: escape measurements | 0.0 | 0.0 | 1.0 | 0.2 | 0.4 | 0.3 | 2.3 | 1.6 | 1.0 | 2.0 | 2.6 | 6.0 | 2.9 | 17.6 | 8.1 |
| refinement: the rest | 0.6 | 0.4 | 0.3 | 3.9 | 12.1 | 4.2 | 3.1 | 2.9 | 1.0 | 2.6 | 2.5 | 2.4 | 3.2 | 1.4 | 1.6 |
| repair and align | 3.7 | 0.5 | 0.1 | 0.0 | 3.5 | 56.7 | 7.5 | 7.1 | 9.1 | 25.1 | 7.6 | 8.8 | 12.4 | 15.8 | 8.7 |
| outside the phases | 5.4 | 0.6 | 0.2 | 0.4 | 0.3 | 1.8 | 0.7 | 0.2 | 0.3 | 1.1 | 1.1 | 0.4 | 1.0 | 1.3 | 0.5 |

- **The anneal** is 22 to 74 % of every board but video and the
  SmartSpin2k panel: its overlap sums (up to 39 %, MegaDrive; 34 % DSP,
  29 % SNSP-CPU-01, 26 % Castor) and the crossings term (`before` and
  `after`: up to 26 %, Castor; 25 % DSP, 23 % MegaDrive and Sisu).
  Inside (callgrind, 10 s of work, one thread): MegaDrive's anneal is the
  crossings' `count` (43 % of its instructions), the overlap sum's
  bucket query (`Grid::scan`, 28 %: the query reaches as far as the
  largest far-side part's boxes, hundreds of cells) and the sum itself
  (10 %, `Constraints::bare`'s binary searches 3 % more); DSP-ADAU1452's
  the sum (29 %, its 354 hollow parts' boxes), the count (24 %), the
  wirelength terms after a move (17 %) and the query (9 %).
- **video's legaliser is its crossings** still (57 % of its placement;
  80 % of the legaliser's instructions are `involving`). Half of the
  fresh lines a spot builds (Glasgow 55 to 70 %, video 45 to 60 %) are
  one of their net's kept lines bit for bit, and so are half of the
  anneal's new lines in `after` (MegaDrive 1.5 of 2.8 million).
- **The 0xCB panel's polygon tests**: 9 % of its placement (its global
  placement 36 %); protoesp's 6 %.
- **The legality tests** (protoesp 36 % with its bucket queries 15 %,
  4in1 26 %, 4in1-mini 16 % and 21 % in the refinement) and **the
  refinement's escape term** (4in1-mini 16 %, 4in1 11 %, SmartSpin2k
  10 %) are the third round's ground; **the relation repair** with its
  refinements is 57 % of SmartSpin2k's placement (books 3.3 units a CPU
  unit: cheap work, much of it).
- **The race's ripped count** (each probe's router copied and ripped
  up, after the placement; seed 1 of the dense tier, x1880 with the
  count on): the copies 0.03 to 0.81 s a race (16 to 267 MB), the
  rip-ups 0.18 to 10.9 s (MegaDrive 10.9 s, split72_left 8.0 s,
  SNSP-CPU-01 4.9 s, the 0xCB panel 3.1 s): the rip-up, not the copy, is
  the race's cost, and it is the scan of every net's conflicts after
  every rip-up.

### The changes (bit-identical)

Each its own commit, each the same computation: the same values in the
same order and the same work booked, a unit test against the direct
computation each (mutation-checked: the tests fail with a unit of work
more, a margin dropped, a box left out of a reach, a version or a
subtraction left out), and, where a cache or a shortcut is new, checked
against the direct computation at every use with
`PCB_PLACER_CHECK_OVERLAP=1` (debug builds always).

1. **The anneal's overlap sums take far pairs by their boxes**
   (anneal.rs `far_work`, `extents_apart`, `pair`). A pair of the sum
   compares boxes (rectangles, pads, hollow boxes, courtyards, far
   sides, far courtyards, a panel frame's pass), each grown by at most
   a margin (the edge margin, half the spacing, a halo, a share of the
   spacing: `far_grow`). Where the part's box around all of them (at its
   pose, `PartBoxes::extent`, with its rectangle) and the other part's
   reach about its body centre (`reaches`: its rectangle and boxes at any
   pose, once per part; the ninth commit) lie farther apart than twice
   that and 10^-6 mm, every comparison gives 0.0, and 0.0 added to the
   sum leaves its bits; the work the pair books (a unit per comparison,
   its lists' lengths) depends on the two parts alone and is booked from
   them (`far_work`, the same branches as the sum). The pair's sum is
   added box by box to the running sum as before (a first version added
   the pair's own subtotal: another rounding, caught by the placement
   identity on the 0xCB panel). Not for opposite sides with
   `anneal_far_side_gaps`. With main's frames rule (x1883) the frame pass
   is in the pair too (its whole bodies' comparison and the connector's
   list). Test: random boards of every kind of part (hollow, far sides
   and courtyards, cutouts with outlines, copper graphics, marks, linked
   pairs, shared halos, frames, boxes far beyond the body), both
   far-side models, parts moving: sums and work against the sums with
   the skip off.
2. **The race's rip-up from a heap of upper bounds**
   (pcb-router coarse.rs `rip_up_most_conflicted`). `rip_up_standing`
   looked for the most conflicted net by counting every net's conflicts
   after every rip-up (up to 11 s a race on MegaDrive, 8 s on
   split72_left, 2026-10-28). A rip-up takes branches away, so the
   occupancy only falls and no net's count rises: each net's last count
   is an upper bound, the top is counted again and, still at its bound,
   is the most conflicted (ties to the larger net id, the scan's
   maximum of (count, net)). The same nets in the same order. Checked
   against the scan at every rip-up in debug builds and with
   `PCB_ROUTER_CHECK_RIP_UP`; test: lattices of crossing nets after short
   negotiations, the nets ripped and the standing against the scan.
3. **The cutouts' and the board's outlines by cells in both axes** (taken
   back by the fourteenth)
   (legal.rs `Outline`). The third round's bands of y became a grid (about
   an edge a cell, as square as the outline's box): an edge in every cell
   its box touches; the near and corner tests from the cells of the box's
   range; the even-odd ray from the point's row, from its column on, each
   edge counted in the first column of its box from there. The same
   answers, the work booked by the outline's length as before. Tests: the
   third round's ragged and lattice rings and board outlines against the
   pass over every edge.
4. **The kept lines' counts kept across moves** (taken back by the
   tenth): per line its cells, lines met and crossings with other nets as
   of the grid's last change in its cells, the anneal's `before` and
   `kept`'s count from them less the crossings among the lines. The
   counts rarely held from one move of a part to its next (the anneal's
   long lines pass many cells, and its kept moves change some of them),
   and the crossings among the moved lines were tested pair by pair:
   MegaDrive's anneal took 10 % more instructions (121.2 G against
   109.8 G).
5. **The anneal's new lines that are a kept line again** (crossings.rs
   `count_fresh`, `kept_twin`). Half of `after`'s new lines have the ends
   of one of their net's kept lines, bit for bit (the tree's edges away
   from the moved pins: MegaDrive 1.5 of 2.8 million): such a line walks
   the same cells, meets the same lines and crosses the same ones; from
   the fourth commit's counts here (the tenth: from `before`'s records).
6. **The legaliser's and the refinement's fresh twins** (`involving`): a
   fresh line that is a kept one again took its kept lines' part of the
   count from the kept count less the other moved nets' kept lines it
   crosses. That subtraction tested every twin against every kept line of
   the other moved nets (the legaliser counts every net, video's large
   ones too): video's legaliser took 35 % more instructions (80.2 G
   against 59.3 G); replaced by the tenth.
7. **The race's copies without the negotiation's history** (coarse.rs
   `copy_for_rip_up`, board_layout.rs). The history (per layer and cell,
   per tile) is read by the searches, the negotiation, the finish and the
   reports, not by the rip-up or the standing; the copy is made with it
   moved out of the probe's router and back. Test: the copy's ripped
   standing is the router's, the router's history unchanged.
8. **The anneal's pair lookups from per-part flags** (anneal.rs
   `PairFlags`): `lies_on`, `bare`, `halo_free`, binary searches of the
   constraints' lists (3 % of MegaDrive's anneal); the lists' own answers
   where one is not ascending. Test: every pair of random boards.
9. **The config reference** (`PCB_ROUTER_CHECK_RIP_UP`; the round's checks
   under `PCB_PLACER_CHECK_OVERLAP`), regenerated.
10. **`before` records its lines' counts for `after`, and the
    legaliser's twins keep theirs per search** (`count_kept`,
    `kept_counts`, `twin_counts`; takes the fourth back, replaces the
    sixth). `before` counts as `count` did (the same tests) and records
    per kept line its cells, the lines it met and its crossings with the
    lines outside the moved nets; `after`'s twins take those while the
    grid and the moved nets are as `before` left them. In `involving` a
    fresh twin's kept-lines part of the count (lines met, crossings
    outside the moved nets) is counted once and kept per kept line while
    the grid and the moved nets stay (a part's spot search, a part's
    candidates); the fresh lines among themselves as before. Tests: the
    exactness test with `after` in check mode (count and work against
    `count`), `involving` against `involving_direct`, mutations of the
    records' validity and the twins' counts.
11. **The far test from the other part's reach** (anneal.rs `reach`):
    bringing every near part's boxes to its pose for the first commit's
    test was 9 % of DSP-ADAU1452's anneal; a part's reach about its body
    centre (its rectangle, and each box's centre distance and half sides)
    is the same at every pose, once per part. Test: the first commit's,
    with boxes reaching far beyond the body (a reach without them fails
    it).
12. **The pair sum generic in its closures and inlined**: the first
    commit's `pair` took its comparisons through `dyn Fn`, an indirect
    call a comparison; DSP-ADAU1452's anneal 88.9 G instructions with
    it, 82.8 G without (92.7 G on x1884).

13. **A twin among a large net's kept lines by their ends**
    (crossings.rs `kept_twin`, `twin_index`): the sixth and tenth
    commits scanned a fresh line's net's kept lines for one with its
    ends; the legaliser and the refinement count every net, and a power
    net's hundreds of lines were scanned for each of its hundreds of fresh
    lines at every spot and candidate: DSP-ADAU1452's placement took 10 %
    more CPU in the layout pairs of the head before this commit (r11:
    395 / 401 s against 433 / 443 s, the layouts' placement). Nets of more
    than 8 kept lines keep them sorted by their ends (and their place in
    the net's list: the first of equal ends, as the scan) while the grid
    does not change. Test: the index against the scan on every net of
    the exactness test's boards.

14. **The outlines by bands of y again** (reverts the third): by cells,
    the even-odd ray walked every column of the point's row from the
    point on (`Outline::inside`: 0.56 G instructions of protoesp's
    legaliser against 0.32 G by bands), and the near tests took as long
    as by bands (0.23 G against 0.25 G): protoesp's legaliser 1 % more
    instructions, the 0xCB panel's polygon tests no cheaper in its pair
    (below).

The instructions of the phases (callgrind, 10 s of work, one thread,
release binaries; the same work, so the same steps):

| phase | x1881 / x1884 / x1885 | after the fifth and sixth commits (r7) | before the twelfth (r11) | before the thirteenth (r12) |
| --- | ---: | ---: | ---: | ---: |
| MegaDrive's anneal | 109.8 G | 121.2 G (+10 %) | 105.8 G (-3.6 %) | |
| DSP-ADAU1452's anneal | 92.7 G | 95.8 G (+3 %) | 88.9 G (-4.1 %) | 82.8 G (-10.6 %) |
| video's legaliser | 59.3 G | 80.2 G (+35 %) | 47.7 G (-19.5 %) | |
| protoesp's legaliser | 12.48 G | | | 12.61 G (r13, +1.0 %) |

### Measured (2026-11-04)

**The placement** (`place-kicad-board` of the task, the race's 16 seeds
and the layout's 600 s of work; pinned pairs on SMT siblings, CPUs 8 and
9 against 24 and 25, one orientation after the other, two threads,
pinned rates, `PCB_ROUTER_NO_WALL_GUARD=1 PCB_ROUTER_FINGERPRINT=1
PCB_MAKER_NO_CACHE=1`, from 01:10 to 03:30 with the nightly from 02:00 and
four other agents: the machine's load 25 to 52). x1885 against the head
(`-r13`): CPU seconds of both orientations, the change (the geometric
mean of the orientations' ratios), peak RSS (MB), the placed board
identical in all four runs:

| board | x1885 CPU s | head CPU s | CPU | x1885 MB | head MB | identical |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| DSP-ADAU1452 | 374 / 373 | 370 / 366 | -1.4 % | 88 / 88 | 97 / 86 | yes |
| video | 631 / 553 | 545 / 478 | -13.6 % | 83 / 85 | 85 / 86 | yes |
| 0xCB panel | 110 / 112 | 107 / 111 | -1.9 % | 161 / 162 | 162 / 164 | yes |
| protoesp | 355 / 365 | 358 / 369 | +0.9 % | 68 / 72 | 70 / 71 | yes |
| glasgow | 753 / 736 | 726 / 717 | -3.1 % | 102 / 102 | 101 / 101 | yes |
| 4in1-mini | 457 / 442 | 462 / 448 | +1.2 % | 86 / 86 | 77 / 76 | yes |
| 4in1 | 703 / 607 | 705 / 618 | +1.0 % | 78 / 78 | 79 / 79 | yes |
| SmartSpin2k | 214 / 207 | 209 / 203 | -1.9 % | 172 / 177 | 173 / 178 | yes |
| MegaDrive | 378 / 412 | 370 / 404 | -2.0 % | 83 / 83 | 84 / 84 | yes |
| SNSP-CPU-01 | 526 / 414 | 518 / 433 | +1.5 % | 170 / 153 | 170 / 172 | yes |
| split72_left | 398 / 382 | 383 / 369 | -3.5 % | 555 / 553 | 556 / 554 | yes |
| Castor | 169 / 164 | 173 / 168 | +2.1 % | 112 / 130 | 113 / 114 | yes |
| k30-SBC | 147 / 137 | 146 / 135 | -0.7 % | 58 / 53 | 54 / 55 | yes |
| jetson | 388 / 371 | 391 / 371 | +0.5 % | 1149 / 1150 | 1149 / 1149 | yes |
| Sisu | 446 / 437 | 453 / 445 | +1.7 % | 199 / 214 | 199 / 214 | yes |

The placement's CPU falls 2.2 % as the geometric mean of the eleven
dense boards, rises 0.9 % on the four fingerprint boards: -1.4 % over
the fifteen. **The round's target (-15 %) is not met.** video falls
13.6 % (its legaliser's fresh twins: the legaliser's instructions -19.5 %),
split72_left 3.5 %, Glasgow 3.1 %, MegaDrive, SmartSpin2k, the 0xCB
panel and DSP-ADAU1452 1.4 to 2.0 %; the anneal-heavy fingerprint boards
and the legality-heavy boards (protoesp, the 4in1 boards) move by -0.7
to +2.1 %: the anneal's instructions fell 4 to 11 % at 10 s of work
(above), a third to a half of the boards' placement, and the outline
commit cost protoesp and the 4in1 boards about 1 % (the fourteenth
takes it back: below). Pairs of these boards differ by up to 2 % from
orientation to orientation and run to run on the loaded machine
(protoesp +0.5 % and +2.4 % in two earlier pairs of intermediate heads).
The peak resident size moves by a few megabytes (the twins' records).

**Taking the outline commit back** (`-r14`, against `-r13`; the same
pinned pairs): 

| board | r13 CPU s | r14 CPU s | CPU | r13 MB | r14 MB | identical |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| protoesp | 340 / 365 | 339 / 358 | -1.1 % | 72 / 70 | 67 / 70 | yes |
| 4in1 | 724 / 717 | 712 / 698 | -2.2 % | 80 / 79 | 76 / 76 | yes |
| 4in1-mini | 400 / 362 | 405 / 359 | +0.1 % | 77 / 77 | 74 / 74 | yes |
| 0xCB panel | 94 / 98 | 94 / 101 | +1.6 % | 161 / 161 | 158 / 159 | yes |

-0.4 % as the geometric mean of the four, within the pairs' noise
(protoesp -1.1 %, 4in1 -2.2 %, the 0xCB panel +1.6 %); the fourteenth
commit is kept for the instructions (protoesp's legaliser) and the
simpler index, not for a measured gain, and the panel's polygon tests
stay the next step for a 2-D index (one that keeps the bands' rows for
the even-odd ray).

**The layout** (`layout-kicad-board`, the task's 1500 s of work; both
orientations at once on CPUs 2-7, 10, 11 and their siblings; the head
`-r13` against x1885): the layout's CPU, the placement's (the process's
CPU from `layout: placing` to `layout: placed`), peak RSS, the placed
board, the result board and every fingerprint line (as a set: their
order between the parallel stages differs from run to run on either
binary) identical in all four runs:

| board | x1885 CPU s | head CPU s | CPU | x1885 placement CPU s | head | placement | x1885 MB | head MB | identical |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| MegaDrive | 2759 / 2717 | 2705 / 2741 | -0.5 % | 429 / 414 | 395 / 415 | -4.0 % | 2029 / 2022 | 2011 / 2019 | yes |
| DSP-ADAU1452 | 3842 / 3914 | 3814 / 3891 | -0.7 % | 372 / 391 | 396 / 398 | +4.2 % | 1301 / 1296 | 1289 / 1288 | yes |
| glasgow | 1970 / 1909 | 1922 / 1916 | -1.0 % | 747 / 713 | 721 / 691 | -3.2 % | 533 / 532 | 527 / 525 | yes |
| split72_left | 4897 / 4853 | 4895 / 4788 | -0.7 % | 430 / 425 | 416 / 410 | -3.3 % | 2738 / 2735 | 2674 / 2739 | yes |
| SNSP-CPU-01 | 2478 / 2431 | 2468 / 2515 | +1.5 % | 544 / 536 | 528 / 528 | -2.2 % | 1870 / 1884 | 1831 / 1819 | yes |
| SmartSpin2k | 1765 / 1824 | 1701 / 1851 | -1.1 % | 248 / 248 | 241 / 243 | -2.5 % | 1261 / 1152 | 1220 / 1121 | yes |
| video | 2071 / 2072 | 1901 / 2022 | -5.3 % | 606 / 618 | 518 / 529 | -14.5 % | 2111 / 2076 | 2065 / 2038 | yes |

The layout's placement falls 2.2 to 14.5 % on six of the seven (-3.8 %
as the geometric mean: video -14.5 %, MegaDrive -4.0 %, Glasgow and
split72_left -3.2 and -3.3 %, SmartSpin2k -2.5 %, SNSP-CPU-01 -2.2 %)
and rises 4.2 % on DSP-ADAU1452 (+6.5 and +1.8 %: its placement pair
above falls 1.4 %; the layouts' placement spans are shorter and noisier
under the load); the layout 1.1 % (video -5.3 %).

The race's ripped count (each layout's `placement race: the ripped
count` line, wall clock under the load): the copies' largest growth falls
by the history's share, 10 to 29 % (video 422 to 381 MB, split72_left 272
to 227 MB, MegaDrive 265 to 233 MB, SmartSpin2k 155 to 110 MB), their
time stays 0.07 to 0.7 s; the rip-ups (0.3 to 8.8 s) move within the
load's noise (video 3.99 / 3.15 s off, 2.80 / 1.61 s on; split72_left
8.80 / 7.86 and 8.24 / 7.96 s): the scan of every net after every rip-up
was not their cost; the restamps of the ripped nets and the standing
are. Wall clock only: no work is charged, the layouts' budgets do not
move.

**Identical.** Every placement of the pairs (fifteen boards, all four
runs) and of the layout pairs (placed and result boards, fingerprint
lines). The head against x1884 at 60 s of work on all fifteen boards (and
the head before the thirteenth commit, `-r12`, and `-r11`'s on six at
30 s): the same placements. Each commit on its own: `-c1` to `-c6`
(the first six on main 84ba6893) against x1881, `-r1b` (the first on
main 9d6844aa, with the frames rule of x1883 in its pair sum), `-r9`,
`-r10`, `-r11` against x1884, placements at 30 s of work of the 0xCB
panel, SmartSpin2k, protoesp, video, DSP-ADAU1452 and MegaDrive: the
same board from all of them (the router commits change no placement).
In check mode (`PCB_PLACER_CHECK_OVERLAP=1 PCB_ROUTER_CHECK_RIP_UP=1`)
on the 0xCB panel, protoesp, Glasgow, video, DSP-ADAU1452, MegaDrive and
SmartSpin2k at 60 s: no difference, and the placements those of x1885 in
check mode. The route-mode fingerprints (Castor, k30-SBC, jetson, Sisu
from build/github-layout-v16, a quarter budget, two threads): every fingerprint line
and the routed boards identical on x1884 and `-r11` (Castor's still
dad3a6cb3787; route mode places nothing and runs no race).
The rebased head (`-final`, on 67ad67a2) against x1888 at 30 s on the six
boards: the same placements. `scripts/check.sh --fast` passes.

**The work clock**: the work booked is the same by construction (each
change books what the computation it replaces booked).

### Candidates not taken

- **The anneal's bucket query** (MegaDrive: a quarter of its anneal's
  instructions after the changes, `Grid::scan`; DSP-ADAU1452 9 %): its
  reach is the largest far-side part's, so every query spans hundreds of
  cells. With the far pairs adding 0.0, only the near pairs' order
  matters, and it can be rebuilt from each item's first cell in the
  query's row-major scan and its place in that cell's list; but the work
  books the distinct items of the whole range and the far pairs' work by
  their kinds, which needs counts of the items by kind over the range
  (2-D prefix counts kept through every move, by inclusion and exclusion
  of the four sides). Not tried this round; the next step for the anneal.
- **The kept lines' counts across moves** (the fourth commit, taken back):
  see above. Counts that hold across the anneal's moves would need the
  grid to change less than it does.
- **The outlines by cells in both axes** (the third commit, taken back by
  the fourteenth): the even-odd ray then walks every column of a row; a
  grid for the near and corner tests beside the bands' rows for the ray
  would keep both, but the near tests took as long by cells as by bands
  on protoesp, and the 0xCB panel's placement did not move with either
  (the panel's frames rule of x1883 changed its placement and its share
  of polygon tests since the profile). Not tried.
- **The race's rip-up beyond its scan**: the heap removed the scan of
  every net after every rip-up, and the rip-ups' wall clock did not move
  measurably (3 to 9 s on the largest boards, under the load): their cost
  is the ripped nets' restamps and the standing's pour analysis. Not
  measured further (wall clock only, no work charged).
- **A twin's count less the other moved nets' lines** (the sixth
  commit, replaced by the tenth): the subtraction
  tested every twin against every kept line of the other moved nets;
  video's legaliser took 35 % more instructions (80.2 G against 59.3 G).
- **Copy-on-write of the race's routers**: the probes' routers route on
  (the kept one) or are dropped; a copy that shares the occupancy until
  the rip-up writes it would need the occupancy, the stamp lists and the
  nets' branches behind shared pointers in every router. The copies
  without the history (the seventh commit) and the rip-up from a heap
  (the second) are the cheap part of it.
- **The anneal's wirelength terms after a move** (DSP-ADAU1452 17 % of
  its anneal's instructions, `net_length`): the pins of the moved part's
  nets turned to their poses; the other parts' pins could be folded once
  per net as the refinement's `MovingNets` does, but the anneal's nets
  change with every kept move. Not tried.
