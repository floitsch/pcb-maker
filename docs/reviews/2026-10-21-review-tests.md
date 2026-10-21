# Review 2026-10-21: what the tests protect

Read-only review of main at 20956b3. Nothing was built or run. The timings come from
today's `build/{dim1,fence,fence2,ladder3}/check.log`.

## Summary

- **The fingerprint test checks determinism, not behaviour.** `tests/router_fingerprint.rs`
  routes one board, `benchmarks/fence/v4-2layer`, at 1 and 4 threads and requires equal
  fingerprints. It stores no expected values. The four-board fingerprints in the changelog
  (Castor, k30, jetson, Sisu) are manual runs. A deterministic regression, such as vias back
  in SMD pads, passes every test.
- **No test checks the router against KiCad.** GitHub's runners have no `kicad-cli`. So the
  walkthrough job always skips (as a notice), and so does
  `with_kicads_check_the_exit_code_is_the_verdicts`. "Router complete equals KiCad
  connected" is checked only by the census and the tiers. The unit tests write KiCad's
  answers into their comments.
- **The placer's order test most likely passes by a race** (see below).
- **23 Rust tests never run, in check.sh or in CI:** pcb-congestion (5), pcb-topo (8),
  tools/config-reference (4), and the root binary's unit tests (`src/agent_cli.rs` 2,
  `src/freerouting_backend.rs` 4).
- **check.sh is not slow:** 2.5 to 4 minutes warm. The walkthrough takes 31 to 83 s of it.

## What runs where

| Step | check.sh | CI | Note |
|---|---|---|---|
| lib + tests of router, kicad, placer | yes | yes | 28–78 s including the compile; placer `tests/placement.rs` 7.5–20 s |
| cli_exit_codes, native_verification, config_reference | yes | yes | the KiCad case skips in CI |
| router_fingerprint | yes | yes | 5–11 s, one board |
| agent_walkthrough | yes | job of its own, **always skipped** | 31–83 s |
| pytest (agent-tasks, github, expansion) | yes | yes | under 6 s |
| congestion, topo, config-reference lib, src unit tests | no | no | |

## (1) Today's packages: what is checked and what is not

| Package | Checked by tests | Not checked |
|---|---|---|
| config reference (98efa4a) | `config_reference.rs`; `kicad_older_than_10_is_warned_about` | the generator's own 4 unit tests (never run) |
| ladder fault isolation (35456a1) | 17 tests in ladder_tests (scripted `Fake`); CLI panic tests; grid capacity | the probe hook through the binary; the "identical boards" refactor claim (measured by hand) |
| router rule features (ffa49a5) | rule_tests 5; the dru and project_rules unit tests | **the plumbing**: `core_config` copying `net_rules` (route.rs) and main.rs's merge, one line each. Nothing routes a `.kicad_dru` board end to end |
| lowering gaps 1 and 2 (837b5f6, 401ab2f) | lowering_gap_tests, one per item; the stub check | agreement with KiCad (comments only) |
| connectivity agreement (3223282) | 11 unit tests, one per cause | complete against KiCad's 0 unconnected (census and tiers) |
| pour model (642e28c) | 4 unit tests | the starved-thermal census (experiments/) |
| wall guard (6825ce3) | ladder_tests (spokes counted, first-iteration gate); the stretch test; complex outline; run.py timeouts | **the manual spokes' reuse of pour analyses** (a cache never compared with a fresh analysis); the spokes' wall deadline |
| strict rule 2 (36f24ba) | test_run.py and test_compare.py on synthetic reports | — |
| all pour work charged (8608a51) | `route_mode_charges_all_the_work`; build and lowering price tests | the attribution switches; the calibration on real boards |
| free dimension (ce8ce30) | 12 unit tests (geometry, band, zones, rule areas, infeasible sizes, ladder choice); 1 CLI test | through the binary only the "no size clean" path. A clean size stopping the probes and its confirmation are unit-tested only. The rates passed to the children are not asserted |
| probe overhead (bbdb205) | the polygon index against plain tests; statics sharing on one board; the ladder's step cost | **that the shared-statics key is complete**. ded766d had to add `no_via_pads` to it in the rebase |
| fence report (ded766d) | rule_tests 4 (ring out of SMD pad, own net beside, TH pad, hop); lowering list with and without `via_in_pad`; plane rule area; refine budget; keepout warnings; decoupling; the score's vias in pads; silkscreen outline (pcbnew stand-in) | **no routed board is checked for 0 vias in SMD pads**; `via_in_pad: true` end to end; `verify` checks neither same-net hole spacing nor vias in pads |

**What the fingerprint board covers.** `v4-2layer/fence` has one B.Cu GND zone, 276 SMD
pads, 60 through-hole and 2 NPTH. It therefore runs the pour model and the no-via-in-pad
maps, but checks only that they are deterministic. It has no `.kicad_dru`, no net tie, no
jumper group, no padstack and no Margin graphic. The strict rule and the free dimension lie
outside route mode. `net_tie` appears nowhere in the sources or the tests.

## (2) Flaky by construction

1. **`order_tests::a_placement_does_not_depend_on_the_order_of_the_file`.** The cause is a
   likely mechanism; it has not been reproduced.
   - 3d28004 added a process-global cache, `canonical_order::SOURCE_ORDERS`. It is keyed by
     a digest of a footprint (its reference and canonical graphics) and holds the file's
     graphics order, which the writer restores.
   - The forward and backward boards share digests, and the last parse wins.
   - The sibling test `the_lowering_does_not_depend_on_the_order_of_the_file` parses the
     same two boards in the same process, at a moment set by thread scheduling.
   - The two outputs match only when the sibling's backward parse lands inside the forward
     placement.
   - Prediction: run alone, it fails every time. The fence check.log has it passing; the
     fence changelog has it failing.
   - Fix: carry the source orders with the parsed board instead of in a static.
2. **`a_free_dimension_searches_the_sizes_and_keeps_one_layout`.** Its `binary()` removes
   `PCB_WORK_RATE` and leaves the wall guard on, with two 2 s probes side by side. Heavy
   load can cut them (exit 3 where 1 is asserted). Pin the rate and set
   `PCB_ROUTER_NO_WALL_GUARD`, as the walkthrough does.
3. **`test_import_pending.py::test_whole_job_timeout_kills_native_child_and_grandchild`**
   has both races fixed today in test_triage: a 1 s timeout before the pid file exists,
   and `exists()` followed by reading `/proc/<pid>/stat`. It also asserts
   `wall_seconds < 3`.
4. Wall-clock bounds that are generous but still depend on load: test_convert
   `seconds < 2`, TierLock's 5 s, the triage memory gate's waits of 3 s, and the
   walkthrough's `elapsed < 60`.
5. **Memory state.** The fingerprint test does not pin it, and the ladder reads
   `MemAvailable` for side-by-side routers and the finer-pitch gate. The risk is low on this
   small board. Set `PCB_MAKER_MEMORY_MB` in the test.

Not flaky: the seed-race test (its sleeps only reorder threads), the router's pour-work
test (deadline an hour ahead), and the ladder tests (arithmetic over `Fake`).

## (3) Speed and a fast gate

check.sh's tests take about 2 minutes, and the rest is compiling (`cargo test --release` and
`cargo build --release` link separately). The parts that are slow and sensitive to load are
the walkthrough and placement.rs.

- **Gate (every commit, about 30 s of tests):**
  - the lib tests;
  - cli_exit_codes, native_verification, config_reference, router_fingerprint;
  - pytest;
  - the orphaned 23 tests (add `-p pcb-congestion -p pcb-topo -p config-reference --bins`);
  - fixture tests 1 to 4 and 8 to 10 below.
- **Nightly or pre-push:**
  - the walkthrough;
  - placement.rs;
  - the kicad-cli fixture tests (5 to 7), in a CI job with KiCad 10 actually installed
    (`kicad/kicad:10.0` image or the PPA).

## (4) Checked only by the tiers

- KiCad agreement: complete against unconnected, and starved thermals.
- 0 vias in SMD pads on routed output.
- Pair and last-matching-rule clearances end to end.
- Hole spacing between vias of one net.
- Net ties.
- The work-to-wall calibration.
- Byte identity after refactors.
- Free dimension's clean path through the binary.
- The silkscreen repair on real pcbnew.

Small fixture boards with committed goldens should check these; the tiers stay as
measurement.

## (5) Dead tests

- `project_rules::census_of_custom_rules`: `#[ignore]`, gated on an environment variable,
  prints and asserts nothing. It is a tool, so move it to experiments/.
- In CI, the 3 walkthrough tests and the KiCad case of cli_exit_codes return early and
  report "ok".
- The 23 tests that are never invoked.
- A scan found no other Rust or Python test without assertions.

## Proposed fixture tests

Two committed fixtures:

- **F1**: 2 layers, about 12 parts, 30 × 25 mm, with:
  - a QFN-16 with an exposed pad and thermal vias;
  - an 0805, a SOT-223, a header;
  - a net tie;
  - a stacked F/B pad pair of one number;
  - a `jumper_pad_groups` footprint;
  - net "14";
  - a copper arc with a net;
  - a hatched GND zone and a +5V zone of equal priority;
  - a Margin graphic, a `gr_text_box` and a dimension on F.Cu;
  - an NPTH slot, a mask-only plated hole `(layers "*.Mask")`, a padstack pad;
  - a `.kicad_dru` with JLCPCB's "Pad to Track" 0.2 then "Min. Spacing" 0.127, and one
    pair rule between two nets.
- **F2**: F1 with a `free_dimension` config and a fake kicad that is clean at 22 mm or more.

Golden files: kicad-cli 10.0.7 DRC JSON of F1 unrouted and of a reference route, and the
fingerprints. They are rewritten under `PCB_UPDATE_GOLDEN=1`.

The tests, ordered by the regressions of today they would have caught:

1. **No via in an SMD pad, no hop.** Route F1 (pinned rate, no wall guard, no native
   check). Assert:
   - `vias_in_pads` is 0 and the exposed pad keeps its thermal vias;
   - no two vias of one net are closer than drill + hole_to_hole;
   - `via_in_pad: true` admits a via in the QFN pad.

   (The fence's 14 vias in pads; Castor's hop.)
2. **Shared-statics key.** Routers on F1 differing in one of `no_via_pads`, `via_in_pad`,
   `net_rules`, keepouts, pitch or classes do not share statics, and their maps equal
   unshared builds. (The missing `no_via_pads` key.)
3. **Rules end to end.** Route F1 through the CLI. Assert:
   - board-router.json lists the pair rule `applied` and "Pad to Track" `no_effect`;
   - the output keeps 0.127 mm from pads and the pair clearance (`verify_with_rules` or
     score-kicad-board on the output).

   (A dropped `net_rules` line; AMLing2's and tuxkeychain's findings.)
4. **Golden fingerprints.** F1's fingerprints at 2 s equal the committed ones, as well as
   1 against 4 threads, with memory pinned. Every behaviour change today would have needed
   an acknowledged golden update.
5. **Connectivity agreement.** The lowering's terminals and connections against the golden
   unrouted report's unconnected items: the stacked pair as 2, the jumper group as 1, net
   "14" routed, the arc a terminal, the net tie as KiCad treats it. With kicad-cli: after
   route, refill and DRC, router complete if and only if KiCad has 0 unconnected.
6. **Pour model.** Predicted starved thermals and pieces equal the golden DRC's (spoke end
   in a clearance, via relief in the hatched zone, equal-priority uuid order).
7. **Lowering zoo.** Obstacle counts by kind (Margin, slot edge clearance, text box,
   dimension, padstack back copper, plated hole without layers) against a golden. With
   kicad-cli, no clearance or edge finding beyond the designer's.
8. **Free dimension, clean path.** F2's probes stop at 22 mm and the confirmation runs at a
   third of the budget. The outline is 22 mm, the band's footprints, zones and rule areas
   move, and the children get the parent's pinned rates.
9. **Order independence without the race.** Forward and reversed F1 in two
   `place-kicad-board` processes give equal poses and canonical outputs. A single process
   writes a footprint's graphics in source order (the reason for 3d28004).
10. **Plane and keepout.** A layout with `add_pours plane: true` on B.Cu and a keepout over
    the fixed header. Assert:
    - one rule area, and no track on B.Cu;
    - the keepout allows footprints;
    - its warning comes first in `next_actions`.
