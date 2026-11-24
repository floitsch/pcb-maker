# Single-net retry: the endgame takes the closes it missed

The window census of the near-miss boards (branch `fund-window`,
`docs/experiments/window-census.md`) found that 13 of the 14 open items it
could close in a small window needed no other net moved: the item's net
alone, routed against the copper the router left, closes it. 7 of them
KiCad accepts, which makes four boards strict passes (split72_left,
capybully, split72_right, DSP-ADAU1452). This package asked why the
endgame missed them, and made it take them.

## Verdict

The endgame already had the single-net retry: the local open repair
(`open_repair/local.rs`: the open net alone against all kept copper, the
pours lowered out of the search, KiCad's refill as the judge). Four things
kept it from closing these items, none of them a search failure:

1. **KiCad's library findings at their cap refused every trial.** The
   checks run without the user's library tables (`kicad_config.rs`), so a
   board of 199 library footprints or more lists `lib_footprint_issues`
   199 times, KiCad's per-type cap; `report_capped` then refused the
   repair. On DSP-ADAU1452 and split72_right the repair found the path,
   KiCad found the trial closed and its findings unchanged, and the trial
   was refused (`native_baseline_report_capped`). On the night of
   2026-11-24 it refused the layout's repair on 17 of the 34 sweep boards
   that reached it.
2. **Route mode had no open repair at all.** The designer tier is route
   mode: DSP-ADAU1452 and split72_right never met the repair.
3. **A layout's final ladder used the work the repair needed** (10 of the
   34 sweep layouts: `insufficient_remaining_work`, 2 of them after the
   fill's reserve).
4. **A layout repaired one net at most.**

The fix: library (and text) findings neither cap a report nor count as new
findings in a repair's comparison; route mode runs the repair after the ladder; a
layout's final ladder leaves it a small reserve; the budgeted repair tries
the open nets one after the other while its work covers them.

## Puzzle 1: split72_left, `open_connections` 0 and an islanded pad

On the 2026-11-18 sweep (binary 3b27f2d0eb93) split72_left's verdict had
`open_connections` 0 and `kicad_unconnected_items` 1: GND between H5's
plated pad 1 and C2_29 pad 2 on B.Cu, 1.6 mm apart.

- **Where the models disagreed.** H5.1 is one label for nine pads: a
  6 mm pad with thermal relief and eight small solid ones around the hole.
  The pour model took the zone connection from the label (solid) instead of
  each pad's own setting, so its fill ran over the large pad and joined
  C2_29.2, whose spoke ends in its relief. KiCad's fill leaves C2_29.2 an
  island of one pad. This is the first of the three rules the
  pour-fidelity package added to the model on 2026-11-23 (59bd64a2,
  router.md "Thermal spokes as KiCad counts them": a pad's zone connection
  is its own); capybully's three census items (SW48.3, SW39.3) are its
  second and third rules.
- **Checked on today's code.** `experiments/pour-fidelity/agreement.py`
  with the binary of 2026-11-24 (96e891b27162) on the 11-18 board: of 387
  pour-net pads, none the model calls connected and KiCad does not;
  C2_29.2 is open in the model and in KiCad (a cluster of one pad). The
  model of 11-18 cannot be dumped the same way (its `dump-pour-map` writes
  no terminals).
- **The verdict reads what it should.** `open_connections` is the router's
  count by definition (verdict.rs: "Connections ... the router did not
  finish, by its own count"); the status and exit code come from KiCad's
  count, and that verdict said "incomplete: 1 unconnected items in KiCad".
  The log's "1 skipped by the router's count" is the ladder's check gate on
  an attempt, not the verdict. Nothing to change there.
- **Why no repair closed it.** The 11-18 binary had no open repair (the
  repairs came on 2026-11-20). On today's binary the layout's repair is
  refused on this board by the library cap (cause 1): today's sweep fails
  split72_left with 3 unconnected items and `local_open_repair.reason`
  "KiCad reached its footprint-library finding limit".

## Puzzle 2: why `repair-kicad-opens --local` missed DSP-ADAU1452 and split72_right

Reproduced on the 2026-11-24 designer tier's result boards with that
night's binary (96e891b27162):

| Board | Search | KiCad's trial | Status |
| --- | --- | --- | --- |
| DSP-ADAU1452 (`/ADAU1452/DVDD`) | 3 objects at 0.025 mm, 19,318 nodes, conflict free | 1 -> 0 unconnected, findings identical | `native_baseline_report_capped` |
| poly_kybd_split72_right (`VDD`) | 7 objects at 0.025 mm, 106,024 nodes | 1 -> 0 unconnected, findings identical | `native_baseline_report_capped` |

Both baselines hold 199 `lib_footprint_issues` warnings ("The current
configuration does not include the footprint library 'Capacitor_SMD'"),
and nothing else capped. KiCad 10 caps each finding type on its own
(`DRC_ENGINE`'s per-code limits; the code already relies on that for
clearance's extended cap), and a library finding compares a footprint with
its library: added copper cannot change it. The refusal protected nothing.
The same binary with the fix accepts both (1 -> 0).

Besides, neither board ever met the repair: they are route-mode results,
and `route-kicad-board` ran no open repair.

**The budget starvation is real** in layouts. In the 2026-11-24 sweep, of
the 34 layouts that reached the repair (KiCad unconnected after the final
board):

| Outcome | Boards | Near misses (1 to 7 unconnected) |
| --- | ---: | --- |
| refused, library cap | 17 | jetson-nano 1, Hub 2, keyboard 2, motherboard 3, split42_left 3, split72_left 3, Sisu 4, vaio_re 5, link 5, split42_right 5, Quanta75 6 |
| skipped, `insufficient_remaining_work` | 8 | MIDAS 1, protoesp 3, katia 3 (and module 8) |
| skipped, after the fill's reserve | 2 | 0xCB pcb-panel 3 |
| searched, no path, no endpoints or no legal access | 5 | RP2040_base 5, SUMEC 5 |
| accepted | 2 | WLED_Matrix 1 -> 0 |

The skipped ones all ran a final ladder to the end of the layout's work
(MIDAS: a final ladder budget of 1154 s, the board's work 1164 s).

## The fix

- **Copper-independent findings** (`open_repair.rs`,
  `COPPER_INDEPENDENT_TYPES`: `lib_footprint_issues`,
  `lib_footprint_mismatch`, `text_height`, `text_thickness`; the text types
  cap jetson-nano's and link's reports): `report_capped` and the silk
  cap's proof ignore them at their cap (`copper_finding_types_at_cap`), and
  `adds_findings` leaves them out of the comparison (a capped list may name
  another subset of them). Every
  other finding still counts, and another type at its cap still caps.
  Every repair that calls `report_capped` runs on such boards now.
- **Route mode** (`board_router/route_open_repair.rs`): after the ladder,
  the solid thermals, the necks and the kept-copper trial, where KiCad
  finds connections open, `repair_layout_opens` on the board kept: each
  open net alone against all the kept copper (`lower_for_local`: pours not
  connected, BGA fan-outs and vias in pads off, foreign exclusive planes
  reserved), shortest native witness first, up to four nets, each kept only
  where KiCad's refilled check finds fewer unconnected items and no new
  finding. Its search work is what the ladder left, at least 20 s; its
  checks count as no work, as route mode's other repairs after the ladder.
  `board-router.json`'s `open_repair` and the log's `open repair:` line
  report it. A layout's own ladders (`kicad_kept`) skip it.
- **Layouts** (`board_layout/ladder.rs`, `additive_repair::reserve`): a
  board KiCad finds open after the first route leaves the repair the fill's
  checks, two checks and 10 s of search from the final ladder (as a
  complete board leaves the fill's reserve), and the neck repair does not
  take it.
- **Several nets** (`open_repair/local.rs`): the budgeted repair searches
  the next open net while its work covers that search and a check for each
  proposal waiting (`search_limit_after`), up to `MAX_TRIALS` (4).

Determinism: the searches are bounded in work (`deadline_work`) as before;
the local repair's 20 s wall guard is unchanged.

## Measurements

**The two designer-tier boards in route mode**, as the nightly ran them
(`route-kicad-board` on the placed project with its `router.json`,
`PCB_WORK_RATE=3.15e6,150e6`, 2 search threads):

| Board | Nightly 11-24 (96e891b27162) | This branch |
| --- | --- | --- |
| DSP-ADAU1452 | incomplete, 1 unconnected (`/ADAU1452/DVDD`) | the ladder leaves the same item; `open repair: accepted, 1 -> 0, 3 objects` (2 checks, 1.8 s of search work); verdict complete, exit 0 |
| poly_kybd_split72_right | incomplete, 1 unconnected (`VDD`) | the ladder leaves the same item; `open repair: accepted, 1 -> 0, 7 objects` (2 checks, 0.6 s of search work); verdict complete, exit 0 |

**The census's 42 boards, the repair alone**
(`repair-kicad-opens --local` on each result board, KiCad as the judge;
`build/scratch/fund-retry/scripts/standalone.py`):

| Binary | KiCad unconnected | Boards at 0 | Boards improved |
| --- | --- | ---: | ---: |
| main (96e891b27162) | 116 -> 111 | 1 (capybully) | 3 |
| this branch | 116 -> 107 | 4 (capybully, split72_left, split72_right, DSP-ADAU1452) | 7 |

The four the branch adds: split72_left's GND (the census's 4-segment
answer), split72_right's VDD, DSP-ADAU1452's DVDD (the census's three
single-net SAT answers KiCad accepted on these boards), and RP2040_base's
`/QSPI_SS_2` (7 -> 6), whose census answer KiCad rejected for a dangling
track and whose repair KiCad accepts. Of the census's seven accepted
items, capybully's three and Sisu's +3.3V (4 -> 3) close on both binaries
(Sisu's on a path of its net alone; the census's answer moved two other
nets); WLED_Matrix's `Net-(LDO1-Vin)` (5 -> 4) too. On main, 27 of the 42
baselines had the library findings at their cap, and 5 trials that held
copper were refused for it; on the branch 1 trial is refused as capped,
and the boards still capped are capped by another type (split42_right: 199
`silk_over_copper`; module: 499 `clearance`; power: 199 `silk_overlap`).

**The evaluation-representations study's 11 KiCad-accepted single-net
paths** (branch `fund-eval-representations`,
`experiments/eval-representations/whyopen.py --verify` over all 117 census
items; 5 beyond the census), against the branch's repair on the same
boards:

| Item | The study | The branch's repair |
| --- | --- | --- |
| split72_left GND (sweep) | a via | closed (0.254 mm on B.Cu) |
| capybully GND x3 (sweep) | a via and two segments each | closed, all three (on main too) |
| split72_right VDD (designer) | 5 segments, a via | closed |
| DSP-ADAU1452 DVDD (designer) | necks to 0.127 mm | closed, necked as the router necks at a pad: 1.17 mm at 0.127 mm of a 0.5 mm class |
| Sisu +3.3V, item 2 (sweep) | necks to 0.1 mm | closed (on main too): 0.87 mm at 0.1 mm of a 0.3 mm class |
| WLED +3.3V (sweep) | one 2.8 mm segment | **not tried**: KiCad names the open by a teardrop it generates at refill |
| WLED Net-(LDO1-Vin) (designer) | 3 segments | **not tried**: a teardrop witness again; named with `--net`, `native_endpoints_unavailable` (the teardrop contact does not resolve) |
| SUMEC +12V x2 (designer) | necks a 2 mm class to 0.1 mm for 0.6 mm | **not taken**: `component_without_legal_access`, "a 0.100 mm track may end at (158.975, 87.521), but no node within 2 mm admits the class's 2.000 mm track" |

So the branch takes 7 of the 11 (the sweep's WLED `Net-(LDO1-Vin)`, which
the study does not list, closes too). Of the study's three router defects:

- (a) **confirmed**: the neck rule wants a node of the class's full width
  within 2 mm of the pad and does not count the same-net copper beside it;
  the repair's own diagnosis says so on SUMEC's IC2.11 and IC3.11.
- (b) the pad escape "found and not kept" at WLED's LDO1: not checked;
  the repair does not reach WLED's items for another reason first (the
  teardrop witnesses).
- (c) capybully's same-numbered pads on two layers as one terminal: not
  checked; the repair closes capybully's three items on both binaries.

**Necking.** The retry necks only as the router necks at a pad (the
class's neck width within the neck zones), and it does not neck a power
class below that: a 2 mm class at 0.1 mm for 0.6 mm (SUMEC's +12V) carries
a fraction of the current the designer's class asks for, and KiCad's
minimum track width does not judge that. Of the branch's 7 accepted closes
on the census boards, 2 neck (Sisu 0.3 -> 0.1 mm for 0.87 mm, DSP-ADAU1452
0.5 -> 0.127 mm for 1.17 mm), both at a pad and both as the routes of the
same nets elsewhere on those boards may. The study's SUMEC paths (2 of its
11) are the necks the retry refuses.

**Remaining single-net misses**: the teardrop witnesses (WLED, 2 items:
the retry identifies an open only by KiCad's endpoint UUIDs, and a teardrop
KiCad regenerates at refill has none on the board; the named teardrop
resolution fails on these), and the walled pads of the neck rule (SUMEC).
The study's design note applies to both: search from the island KiCad's
connectivity names to any copper of the rest of the net, not between
KiCad's two named points.

**The endgame tier** (`run.py --tier endgame`, 38 frozen near-miss boards
of the 11-18 sweep and the 11-24 designer tier, 3 x 2; the route resumed
with its copper kept, then `repair-kicad-opens --local`): main's binary
(96e891b27162, the endgame package's `base-a`) against this branch's
(99d1b02f, before the text findings joined the exemption):

| | Unconnected | Boards closed some | Boards opened some | Strict |
| --- | --- | ---: | ---: | ---: |
| main | 99 -> 81 | 11 | 0 | 3 (capybully, DSP-ADAU1452, split42_right) |
| branch | 99 -> 79 | 13 | 0 | 5 (+ split72_right, split72_left) |

The only boards that differ are the two the branch adds; every other
board ends at the same count. Inside `route-kicad-board` the new repair
accepted on three rows (split72_left GND 1 -> 0, split72_right VDD 1 -> 0,
WLED_Matrix@sweep `Net-(LDO1-Vin)` 2 -> 1, which main's run closes in its
standalone repair after the route instead), each with 2 KiCad checks and
under 1 s of search work; DSP-ADAU1452 the resumed route closes before it.
Of its 35 other rows, 16 found no local path, and 9 were refused as capped
by a copper-dependent type at its cap beside the library findings (protoesp
and motherboard: `solder_mask_bridge`; split42_right: `silk_over_copper`;
jetson-nano: `text_height`; link: `text_thickness`; the last two now pass
as copper-independent, commit 046e82c1, not yet measured on the tier).

## Guards

- `bash scripts/check.sh --fast` (at 99d1b02f, under the machine's load):
  every Rust test passes, the fixture goldens included (f1 routes complete,
  so the route-mode repair does not run there); 2 Python failures in
  `test_tier_estimates.py`, the TMPDIR workaround's known artefact.
  Workspace clippy (`--all-targets`, warnings as errors) and
  `cargo fmt --check` clean; `scripts/check_links.py` clean.
- New tests: `library_findings_at_their_cap_neither_cap_nor_block_a_trial`,
  the waiting proposals' search limit, and
  `an_open_net_is_routed_alone_and_adopted_with_kicads_evidence` (route
  mode's repair with kicad-cli).
- Queued behind the load gate when this was written (see the hand-off):
  the endgame tier again on 046e82c1, the fingerprints, the expansion tier
  (6 x 1), the short tier (layouts change: the final ladder's reserve), and
  the repair alone on the 11-24 sweep's 17 near misses.

## Reproducing

```
# the repair alone, per binary (build/scratch/fund-retry/scripts/standalone.py)
python3 -I standalone.py items.json out main=<binary> new=<binary> --jobs 3
# the endgame tier
benchmarks/agent-tasks/run.py out --tier endgame --jobs 3 --threads 2 --binary <binary>
benchmarks/agent-tasks/endgame_report.py before.log after.log
# a designer-tier board in route mode, as the nightly ran it
PCB_WORK_RATE=3.15e6,150e6 pcb-maker route-kicad-board <night>/designer/<task>/placed <id> out <night>/designer/<task>/router.json
```

The window census's items: `build/scratch/fund-window/items.json`.
