# Profiling the router and the layout

How to profile a route, a placement or a layout, where their CPU and memory
go today, what the optimisation rounds changed and which candidates were
rejected. Every change is bit-identical (the same boards, fingerprint lines
and work booked); the rounds' full notes are in git history.

## How to profile

**perf** is not installed on the benchmark machine and there is no root. The
Arch package runs from any directory, as the user:

```sh
mkdir perf && cd perf
curl -O https://geo.mirror.pkgbuild.com/extra/os/x86_64/perf-7.2.11-1-x86_64.pkg.tar.zst
curl -O https://geo.mirror.pkgbuild.com/extra/os/x86_64/libpfm-4.14.1-1-x86_64.pkg.tar.zst
mkdir root && for p in *.zst; do tar -C root -xf $p; done
LD_LIBRARY_PATH=$PWD/root/usr/lib root/usr/bin/perf --version
```

`kernel.perf_event_paranoid` is 2, which allows sampling one's own processes
in user space (`cycles:u`). Several `perf record` sessions at once exhaust the
locked-memory allowance: pass `-m 16`. The release profile has no debug
information and no frame pointers, so build a profiling binary into its own
target directory, then route one placement at pinned rates, without the wall
guard, on two cores:

```sh
CARGO_TARGET_DIR=target-prof RUSTFLAGS="-C force-frame-pointers=yes" \
  CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release
PCB_WORK_RATE=3.15e6,150e6 PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1 RAYON_NUM_THREADS=2 \
  taskset -c 8,9 perf record -F 499 -m 16 -g -o castor.data -- \
  target-prof/release/pcb-maker route-kicad-board \
  build/github-layout-v16/wntrblm__Castor_and_Pollux__mainboard/layout/placed mainboard \
  out/route router.json --reference none          # router.json: {"ladder_budget_seconds": 300}
perf report -i castor.data --no-children --sort symbol --stdio -g none
perf report -i castor.data --no-children --sort srcline --stdio -g none \
  --symbols='<pcb_router::router::Router>::search'
```

- `--sort srcline` names a sample's innermost inlined frame; for its outermost
  line of router/search/ resolve `perf script -F ip,sym,symoff --no-inline -G`'s
  addresses with `addr2line -a -i -f` (without `--no-inline`, minutes per
  thousand samples). Keep the binary at its recorded path until every report
  is written; record to disk, not /tmp (20 to 85 MB a profile).
- A layout: `layout-kicad-board` under `perf record` follows kicad-cli too.
  Give each run its own DRC cache (`PCB_MAKER_CACHE_DIR`, or
  `PCB_MAKER_NO_CACHE=1`); `PCB_KICAD_CLI` can point at a logging wrapper.
- A placement alone: `place-kicad-board <task source> <board> <out>
  <layout.json> --router auto` is the layout's placement byte for byte;
  `PCB_PLACER_DEBUG` prints each phase's thread CPU, `PCB_PLACER_WORK_SECONDS`
  shortens it.

**Without perf**, callgrind on short runs (10 to 15 s of work, one thread;
`--collect-atstart=no --toggle-collect='*anneal*anneal*'` for one phase) gives
instruction counts that compare across binaries regardless of the machine's
load, since the same work takes the same steps. Per-category CPU shares come
from throwaway timing builds, not kept in the tree.

**CPU measurements** pair two binaries on one core's SMT siblings (CPUs 8, 9
against 24, 25; separate cores differ by up to 6 % under load), started
together inside one memory-ledger reservation (`harness_common.admit`, then
`PCB_MAKER_LEDGER_INSIDE`), then swapped. Count the process's own utime +
stime (kicad-cli left out), report the geometric mean of the two
orientations' ratios, and check the boards' SHA-256, the fingerprint lines
and the work booked for identity.

**Check mode** (`PCB_PLACER_CHECK_OVERLAP=1`, `PCB_ROUTER_CHECK_RIP_UP=1`)
compares every cache and shortcut with its direct computation at every use. A
check books no work and draws no random number, so the placement and routes
are those without it. `check_mode_places_as_without_it`
(crates/pcb-placer/src/lib.rs) and `the_place_check_places_alike_in_check_mode`
(tests/fixtures.rs) hold this; a new check must pass them.

## The work clock

Every budget is work: search expansions at `expansions_per_second` and placer
units. Where the rates are pinned (`PCB_WORK_RATE`, as every tier and run.py
pin them at the reference's 3.15e6 and 150e6), a faster binary does the same
work in less CPU. Where they are measured (`calibrate` in
crates/pcb-kicad/src/work_rate.rs), the rate is the reference's scaled by a
canned search's speed against `REFERENCE_CANNED_EXPANSIONS_PER_SECOND` (6.9e6):
a router change that speeds up the canned search raises the measured rate, and
an unpinned run spends the saving on more search, with other results. The
first router round raised it by 12 to 16 %, the third by about 2 %, the
second not at all. `calibrate-work-rate` measures it.

## Where the route's CPU goes

Route mode on main 59025fb5 (2026-11-05, before the fifth router round's
changes), a timing build, 300 s of work, two search threads; shares of the
router process's CPU, nested as the code nests them:

| category (% of the route's CPU) | Glasgow | 4in1 | jetson | k30-SBC | Castor | Sisu |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| M expansions per CPU second of search | 2.79 | 2.88 | 3.20 | 2.76 | 2.66 | 2.90 |
| search | 86.8 | 90.6 | 83.3 | 84.8 | 79.2 | 79.7 |
| tile planner (`plan_corridor`) | 4.5 | 2.5 | 7.1 | 6.8 | 2.6 | 5.0 |
| stamping (`stamp`, `unstamp`) | 4.3 | 2.5 | 3.0 | 4.8 | 6.1 | 9.9 |
| pour analyses | 1.06 | 1.33 | 0.19 | 1.35 | 8.48 | 0.76 |
| hard repair (`resolve_remaining`) | 6.3 | 10.6 | 5.4 | 1.2 | 0.0 | 12.9 |
| trouble repair | 1.7 | 2.5 | 2.7 | 0.0 | 0.0 | 3.7 |
| clean-up | 13.3 | 2.8 | 6.9 | 18.2 | 33.0 | 7.2 |
| via reduction | 0.0 | 0.0 | 0.0 | 19.3 | 37.8 | 0.0 |
| spoke repairs, straightening, stitching | 0.10 | 0.81 | 0.21 | 0.92 | 11.4 | 0.22 |
| lowering with the via-in-pad plan | 0.12 | 0.11 | 1.40 | 0.04 | 0.06 | 0.26 |
| final pour measure (`measured_board`) | 0.27 | 0.58 | 0.77 | 0.39 | 0.96 | 1.32 |

The search is memory-bound (loads of nodes and occupancy a row away, the
heap's levels, the popped node; all prefetched), its heuristic 6 to 8 % of its
instructions. The tile planner, a Dijkstra without a heuristic, pops most of
the board's tiles in each of 9 500 to 28 000 plans a route (2 to 6 % after the
fifth round). Stamping has no line above 5 % of `Router::stamp`. The verifier,
the parse and the attempt's write are below 0.6 % each.

## Where the layout's CPU goes

The placement on main 84ba6893 (x1881, 2026-11-04, before the fourth placer
round's changes), a timing build at the layout's budget, fifteen boards;
shares of the placement's CPU:

| category | range | largest on |
| --- | --- | --- |
| anneal, all | 22 to 74 % | every board but video and SmartSpin2k |
| anneal: overlap sums | 2 to 39 % | MegaDrive 39, DSP-ADAU1452 34, SNSP-CPU-01 29, Castor 26 |
| anneal: crossings term | 2 to 26 % | Castor 26, DSP-ADAU1452 25, MegaDrive and Sisu 23 |
| anneal: wirelength, relations | 0.2 to 13 % each | split72_left 12 (wirelength), k30-SBC 13 (relations) |
| legaliser: crossings | 0.2 to 57 % | video 57, Glasgow 14 |
| legaliser: legality tests and bucket query | 0.2 to 51 % | protoesp 36 + 15, 4in1 26 + 4 |
| legaliser: cutouts' polygon tests | 0 to 9 % | 0xCB panel 9, protoesp 6 |
| refinement: crossings, legality, escape term | 0 to 21 % each | split72_left 16 (crossings), 4in1-mini 21 (legality) and 16 (escape term) |
| refinement: escape measurements (global route) | 0 to 18 % | jetson 18, Sisu 8 |
| relation repair and alignment | 0 to 57 % | SmartSpin2k 57, split72_left 25 |
| global placement | 0.2 to 36 % | 0xCB panel 36 |

MegaDrive's anneal (callgrind) is the crossings' `count` (43 %), the overlap
sum's bucket query (28 %, reaching as far as the largest far-side part) and
the sum (10 %); the fourth round then cut the anneal's instructions 4 to 11 %.

The rest of a layout is the router's (seven layouts on main 688b59ca: the
search 49 to 86 % of the CPU, the hard repair 2 to 27 %). Lowering and parse
are below 0.1 %, kicad-cli 0.3 to 3.8 %; the neck repair's cost is KiCad's
wall clock (USB_Keypad: 345 s against 5.3 s of the router's CPU).

## Memory

`PCB_ROUTER_MEM_PROFILE=1` (crates/pcb-router/src/mem_profile.rs) prints
`mem-profile:` lines to stderr: `rss high` (a sampler thread reads
/proc/self/statm every 10 ms and prints each new high, every 2 % or 32 MB,
with the phase, the live heap from the `CountingAllocator` and every live
router's parts from `Router::memory_parts`), `phase` (the layout's phases such
as `placing`, `race probes`, `race ripped count`, `moves`, `layout ladder`,
and the ladder's `probes`, `rungs`, `seeds`, `pour measure`), and `peak rss`
at the end with its phase, heap and parts. The profile changes nothing a run
computes. The router's `memory: N MB in maps` line gives one router's maps.

**Where the peak goes** (the second memory round, 2026-11-04, the ten boards
with the largest sweep peaks at the sweep's budget, on main 951b610a's code
with the round's commits): the peaks are 702 to 3156 MB (katia 3156, link
2829, osprey 2755, ATAT1800 1915, Neotron-Pico 1811, split72_left 1718,
SUMEC_MK_IV 1600, CyberKeeb2040 1589, 0xCB pcb-panel 1295, sbc_allwinner_a13
module 702). Every peak but the module's is two or three routers side by side
(the race's probes, the ladder's probes, rungs or seeds), each with its own
occupancy, search records (`hot`), history, stamp lists and search arrays
beside the shared static maps; the search records were the largest part
before the round (katia 1459 MB of 5012). The via-in-pad raster (Neotron's
193 MB) is a lowering's largest allocation.

The first memory round (route mode): the stamp lists as zigzag varint
differences (1.5 bytes a stamp instead of 4), the search arrays shared by the
process (`SearchArrays`), the occupancy in four bits; peaks -13 to -39 %
(Castor 1073 to 661 MB). The second round (layouts): idle search arrays
freed, saved routers and the ripped count without what is rebuilt, the
via-in-pad raster in four bytes a cell, routers side by side at most the
search threads (`at_most_the_threads`), finished probes parked; the ten peaks
-48.8 % (the largest 5160 to 3156 MB), no measurable CPU cost.

## Router rounds

1. **The profile round of 2026-10-25**: the hop test walks back only where a
   via can be (`since_via`), the heap's root waits for the next push
   (`KeyHeap`), the expansion prefetches its neighbourhood. CPU -9 to -12 %.
2. **The second router round**: prefetches of the diagonal steps' and the via
   step's maps and of the next pop's node (`KeyHeap::next_hint`). CPU -4 to -11 %.
3. **The third router round**: a decrease-key open list (`SearchHeap`, the slot
   in the 12-byte search node), so no pop is stale (39 to 41 % were); the
   restamp decodes each list once (`Stamped::iter_ahead`). CPU -3.9 %.
4. **The fourth router round**: the via-in-pad plan's enclosed pads cached by
   their exact inputs, pour windows searched nearest first, the pour map
   labelled by runs. CPU -11.9 % over twelve boards (node_v6_0_0 -45 %).
5. **The fifth router round**: the neck zones as a bit mask (`ZoneMask`), the
   reached target without a hash map (`reached_element`), the tile planner's
   arrays in the thread's scratch (`PlannerArrays`). CPU -2.9 %.

## The second placer round

The first, the layout round, took the turns without libm (`sin_cos_degrees`)
and a near-any relation's anchors only as far as needed (`nearest_violation`):
layout CPU -4.3 to -14.5 %. The second: grid cells without libm's `floor`,
the anneal's wirelength from cached net terms, the refinement's candidates
folding their nets' other pins once (`MovingNets`), relation penalties from
each part's own relations (`PartRelations`), ring searches visiting only the
ring, the relation repair's legality from the parts that may come near
(`Reach`, `Sweep`), the anneal's overlap from cached boxes (`PartBoxes`),
crossings at kept poses from the kept lines (`involving_kept`). Placement CPU
-22.3 % (fifteen boards).

## The third placer round

Bucket queries keep their answers (`Answers`), pairs apart by their boxes take
the legality test's far path untested (`Apart`), polygon tests by bands of y
(`Outline`), fresh trees' crossings without inserting them into the grid,
per-part flags (`Facts`), boxes turned once a pose (`Turned`), the searched
part's frame (`Frame`). Placement CPU -24.5 % (fifteen boards).

## The fourth placer round

The anneal's overlap sums take far pairs by their boxes (`far_work`, `reach`);
new lines that are a kept line again take its counts (`count_kept`,
`kept_twin`); per-part pair flags (`PairFlags`); the race's rip-up from a heap
of upper bounds (`rip_up_most_conflicted`, checked by
`PCB_ROUTER_CHECK_RIP_UP`) and its copies without the history. Placement CPU
-1.4 % (video -13.6 %), short of the round's -15 % target.

## Tried and rejected

- Radix heap for the search: +5 % CPU (keys move through several buckets).
- 8-ary heap, a cache line a level: +1.4 to +5 %.
- Decrease-key slots in their own array: less gain than in the node, 4 bytes a state more.
- The heuristic inlined into every step: +0.9 to +1.9 % (register pressure).
- A summary bit per occupancy word: -3.3 % on a sparse board, +3.8 % on a dense one.
- Decoded stamp lists cached (8 bytes a stamp against 1.5), or fixed-width deltas (1 to 1.5 % to gain).
- No work-rate calibration under pinned rates: its rate is reported, so the reports would change.
- The final pour measure on the route's lattice: the written board's tracks do not fit its maps.
- A\* in the tile planner: equal-cost tile paths could resolve otherwise (other routes).
- The anneal's overlap before a move cached per part: not exact (the query order changes).
- A cached count for the anneal's crossings term: its work depends on every line passed.
- Forgetting only the answers of the cells an item meets: protoesp's legaliser +4.6 % instructions.
- Outlines by cells in both axes: protoesp's legaliser +1 % instructions; reverted.
- Kept lines' counts across anneal moves: MegaDrive's anneal +10 % instructions.
- A twin's count by subtracting the other moved nets' lines: video's legaliser +35 % instructions.
- Copy-on-write race routers: needs the occupancy, stamps and branches behind shared pointers.

Not tried yet: the anneal's bucket query from 2-D prefix counts by kind, a 2-D
index of the cutouts' edges, the anneal's wirelength with the other pins
folded once, search records without the history (a second load a step), pour
masks as bitsets, the race's probes one at a time, the via-in-pad raster in
two bytes, occupancy maps interleaved per cell, a preview of the kept DRC only.
