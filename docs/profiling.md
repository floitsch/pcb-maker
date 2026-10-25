# Profiling the router and the layout

How to take a CPU profile of a route, where the route's CPU goes, and what
the profile round of 2026-10-25 changed (branch router-profile, snapshots
x14380 and x14381 against main's x1820, 23f629e). The layout round of the
same day ("The layout round" below: branch layout-profile, snapshot x14490
against main's x1824, 09299b6) profiled everything else a layout does.

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

