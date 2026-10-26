# Remapper agent diagnostic usability trial — 2026-10-18

The current interface lets an agent find **which nets failed and roughly where**, and native KiCad can identify the missing endpoints. It does not let the agent reliably establish **why a particular route failed**. On this board the two promising diagnostic commands reject an outline accepted by the whole-board router, and the saved report omits unresolved pad identities, effective rule provenance and per-attempt outcomes. The most consequential workaround was a documented width override: it routed three representative signals but emitted tracks below the board minimum, while `describe-kicad-board` continued to report the project-resolved minimum. Native DRC exposed the invalid result.

This is a routing/tool usability investigation, not an electrical or fabrication review. One GPT-6-Astra agent at medium reasoning performed the trial; no further agents were used. **No pcb-maker implementation source was read or changed.** Inputs were CLI help, public documentation, saved logs/JSON, raw KiCad geometry, and KiCad's public Python API/CLI. A broad documentation keyword search incidentally returned changelog matches; no implementation history was investigated. The KiCad skill was used for inspection guidance only. No datasheet, BOM, EMC or SPICE work was undertaken.

## What is established source-free

| Question | Answer and boundary |
|---|---|
| Is the original board feasible? | Yes under the supplied project rules: the designer reference has zero relevant native errors and zero unconnected items. This rules out a claim that layers, outline or placement inherently make the original design unroutable. |
| Which nets remain open? | `board-router.json` lists 19 partial nets in the deep run. It reports 20 unreached terminals. |
| Which physical endpoints? | Not in that report. A separate native DRC supplies 20 missing-connection pairs, all involving pads of U3 or U103. Joining its descriptions/UUIDs to geometry gives coordinates and layers. |
| Where did negotiation struggle? | Ten deep hot spots are on F.Cu around the two RP2040 footprints. They have coordinates/history scores but no net, obstacle or terminal links. |
| Are those pads intrinsically inaccessible? | Unknown. `dead_pads` is empty; this is not proof of accessible escape paths or global feasibility. Both access/cut inspection commands fail on the board outline. |
| Is the emitted deep result otherwise clean? | No. The new native check finds one starved GND thermal, in addition to its opens. Original runs had skipped native verification. |
| Do narrower tracks fix the original design? | No such fix was demonstrated. A diagnostic override made three target nets route but violated the unchanged 0.2 mm board minimum. |
| Is the cause grid alignment, congestion, or ordering? | Not discriminated. Tight escapes and local congestion are supported hypotheses; precise failed search reasons and attempted accesses are unavailable. |

The existing verdict's explicit `native: null`, `kicad_unconnected_items: null` and next-action warning about skipped KiCad are good safeguards. The original zero internal-violation arrays must not be presented as native clearance/thermal verification.

## Ranked changes tied to observed failures

| Priority | Observed friction | Concrete improvement | Acceptance check on this board |
|---|---|---|---|
| **P0** | Finding the 20 unreached terminal identities required native DRC, a separate board description and custom geometry export. `nets[]` only gives counts; `inspect-kicad-connections` counts all pad centers. | Add a **run-aware inspection report**: unresolved connected components, pad UUID/ref/number, absolute mm coordinates, copper layers, actual per-net rules and native evidence links. Distinguish router terminals, native connection pairs and physical pads. | One supported command names U3:52 and U5:6 for `QSPI_SCLK`, reports their coordinates/layers and 0.2 mm rules, and reconciles the deep run's 19 nets/20 router terminals/20 native pairs without conflating the quantities. |
| **P0** | `net_classes` accepted 0.15 mm tracks despite a 0.2 mm project minimum. The output project stores that class, the router emits it, `describe` resolves approximately 0.2 mm, and native DRC finds 35 width violations. | Preflight contradictory overrides before spending route work. Report **requested, project minimum, actually used, emitted and native-required** values with sources. Reject below-minimum overrides unless an explicit design-rule change is requested. | The recorded narrow-four config is rejected with four net names and the `min_track_width` source, or is clearly marked as conflicting before routing. A run query must show that the recorded output used 0.15 mm. Do not make `describe` silently pretend the board minimum changed. |
| **P0** | `inspect-kicad-terminal-access` and `inspect-kicad-routing-cut` both reject this board, while `route-kicad-board` accepts it. Its Edge.Cuts contain eight lines and eight arcs. | Use equivalent geometry/rule support for routing and diagnostic inspection. Until supported, emit a structured capability error naming the unsupported shapes and the supported fallback. | Both commands inspect the unchanged Remapper board with its original rules, or return a specific unsupported-outline result rather than a generic routing failure. They must not require replacing its rounded outline with a rectangle. |
| **P1** | Empty `starved_thermals` in the saved deep report; subsequent native DRC finds U3:57 on B.Cu with one spoke instead of two. Native results remain separate from the old verdict. | Add post-run native-check/reconciliation support without rerouting, with explicit `not_run`, `passed`, `failed`, `error` states, input hash and comparison to the reference. Preserve the original report. | Deep: 20 native opens and one new thermal. Quick: 48 native opens and no new noncosmetic violations. Never infer clean from native CLI exit 0 alone. |
| **P1** | Hot spots provide no object links. Docs say otherwise the board is “too dense for its layer count or size”; verdict suggests larger outline/layers/narrower classes without causal evidence. | Separate observed symptoms from hypotheses and unsupported conclusions. Attach nearby pads, conflicting nets and available access/cut evidence to each hot spot; recommend the next discriminating query before design changes. | A clean reference prevents “fundamentally too dense” conclusions. No automatic narrower-width recommendation below this board's 0.2 mm minimum. Empty `dead_pads` remains an observation. |
| **P1** | Deep logs interleave iteration streams without attempt IDs. Final `routing_seconds` is 59.87 s while run wall time is 271.82 s and work is 352.72 s. A requested 30 s control uses 41.35 s work. | Emit a per-attempt/phase ledger: ID, config/rule hash, grid origin/pitch, seed, work and wall scope, result, selected flag, stop reason, budget accounting. Document a bounded fixed-grid control. | An agent can identify the selected 0.075 mm attempt and explain what ran/skipped and why, using JSON only. Budget overruns identify finishing/other phases rather than leaving the reader to infer them. |
| **P2** | Specialized `--help` prints only usage. A focused 0.05 mm query was not discoverable in user-facing docs. Local reference comparison required another API plus a custom SVG. | Publish config/output schemas and examples for supported diagnostic commands. Add a bounded net/region overlay with reference copper and unresolved endpoints. | `--help` states defaults, units, rule source, outline capabilities and cost. A documented recipe visualizes U3's upper escapes without implementation access or plotting dependencies. |

`describe` reporting 0.2 mm is not itself evidence that its project-rule resolution is wrong: the output project's minimum remains 0.2 mm. The failure is that this view does not expose the override the run actually used, and the routing path accepted a conflicting value. The requested/run/project/native distinction needs to be explicit.

## Taken (2026-10-23, 2026-10-25)

What pcb-maker did with the ranked items; the changelog's "Committed on
2026-10-23 (run diagnosis and override preflight)" and "Committed on
2026-10-25 (the remapper trial's items: run-aware diagnosis, override
preflight, arc outlines, hot-spot links)" have the details, docs/agents.md
("Which nets stayed open") the interface.

- **P0, run-aware inspection: taken.** `pcb-maker inspect-routing-run
  <run-dir> [--net] [--terminal] [--reference] [--native-check]` (2026-10-23)
  writes the proposed join. On this board's deep run with the trial's
  `native-deep.json` as its drc.json and the designer's board as reference
  (2026-10-25, x14440): QSPI_SCLK's groups are U3:52 (138.9, 130.2625) and
  U5:6 (136.25, 125.525), F.Cu, uuids as in the trial, 0.2 mm track and
  clearance with their sources, KiCad's one pair joined to both groups, the
  reference connected (7 segments, 0.2 mm, no vias); at the top the router's
  19 nets / 20 unreached terminals, 39 groups / 20 missing links and KiCad's
  20 pairs plus one starved thermal, side by side and explained apart. Since
  2026-10-25 a pair's track or via item is joined to the group of the copper
  it lies on (the deep run: none left `null`), and a reference net with a
  zone (+1V1, +1V2, +3V3 here) is `connected: null` with the reason instead
  of `false`. `access` and `cause` stay `unknown`.
- **P0, override preflight: taken.** The narrow-four config is refused
  before any work (exit 2, 0.01 s): `net_classes[diagnostic_narrow_four]
  asks for track width 0.15 mm below the project's 0.2 mm
  (board.design_settings.rules.min_track_width in the .kicad_pro) on
  QSPI_SCLK, QSPI_SD2, Net-(U3-USB_DM), CTS1RTS`. With
  `allow_below_project_minimum` the verdict's `rule_overrides` (2026-10-25)
  gives requested, project minimum and source, routed with, the widths
  written, native required and KiCad's findings. On the trial's recorded
  narrow-four output, `inspect-routing-run --net QSPI_SCLK` shows the class's
  0.15 mm beside the project's 0.2 mm and 0.15 mm emitted;
  `describe-kicad-board` keeps the project's 0.2 mm `track_width_mm` and adds
  `class_track_width_mm: 0.15`.
- **P0, access/cut on the unchanged outline: taken as a structured error.**
  Both commands were removed with the research grid router on 2026-10-21;
  called, they now exit 2 naming `inspect-routing-run --terminal REF:PAD` and
  the outlines it reads (lines, arcs, rectangles, polygons, circles,
  curves) rather than "unknown command". Every remaining command that reads
  an outline uses the router's lowering: on this board's arcs and lines
  `describe-kicad-board`, `score-kicad-board`, `inspect-kicad-board` and
  `inspect-kicad-connections` run. No access probe or cut exists.
- **P1, native reconciliation without rerouting: partly taken.**
  `--native-check` runs KiCad's DRC on a saved run and reports `not_run`,
  `passed` or `failed` with the pairs and error counts; input hashes beyond
  the board's sha256 and the reference comparison beyond the pairs are not.
- **P1, hot spots and "too dense": taken.** Hot spots carry their tile
  (`box`), the pads in it (ref, number, net), the nets whose copper crosses
  it and those still in conflict (board-router.json from 2026-10-25;
  `inspect-routing-run` reads them off the result board for older runs).
  The deep run's hottest tile is the 1.2 mm above U3:50-52 with +1V1,
  QSPI_SCLK and QSPI_SD3 in conflict. "Too dense for its layer count" is
  gone from the docs, the skill and the verdict: the open-connections next
  action asks for the inspection first and calls room, layers and narrower
  classes hypotheses where the reference does not rule density out.
- **P1, attempt ledger: taken** (2026-10-23, board-router.json `attempts`);
  config hash, grid origin and seed are not in it.
- **P2, help and overlays: partly taken.** `inspect-routing-run --help`
  lists its fields; no region overlay.
- **The routing hypothesis (tight escapes): tested and taken as a
  switch** (2026-10-25, docs/router.md "Fine-pitch escapes", the
  changelog's "Committed on 2026-10-25 (fine-pitch escapes)"). The cause
  was neither the lattice's phase nor congestion as such: every pad of a
  0.4 mm row with 0.2/0.2 leaves by an escape stub (the centre line keeps
  the clearance only exactly, which the static maps refuse), and the
  stubs stamped discs grown by a snapping margin of 0.525 mm, so every
  pair of neighbouring stubs was in conflict for good. With
  `PCB_ROUTER_DEBUG` on main's 30 s route (x1823, 0.1 mm lattice aligned
  with U3's rows, 27 nets open, KiCad 33 unconnected) all 27 open nets
  ended with their conflicts on their own fine-pitch pads (U3, U103,
  U5, U105, USB101) against the neighbouring pads' nets. With
  `exact_escape_stubs` (stubs along a lattice line stamped as lattice
  nodes, oblique ones by their measured footprint, straight stubs
  first) the board routes complete and clean at 30 s: 120/120, KiCad 0
  unconnected and no new error, 95 vias (x14475). The switch is off by
  default: the expansion tier gains 32 connections and a pass but has
  more KiCad errors (after the net-tie fix of 2026-10-26: 1 off, 5 on,
  four starved thermals at connector pads the pour model misses).

## Minimum implementation for the next trial

First implement the run-aware unresolved endpoint/rule/native join and the below-minimum override preflight. Reuse saved outputs where possible: this should not require an expensive new search. Correct the categorical “too dense” troubleshooting text at the same time. Then make the access/cut diagnostics accept the same unchanged outline and resolved rules as the whole-board router. These changes allow the next agent to select a representative failure and ask a causal question instead of spending most of its effort reconstructing missing context.

A visual viewer, richer route strategies and automatic repair suggestions are secondary. A plain, well-defined JSON answer is sufficient for the next trial.

### Proposed interface and schema — not implemented

The following names/options are proposals, not currently available commands:

```sh
pcb-maker inspect-routing-run <run-dir> --native-check --reference <reference-dir> \
  --net QSPI_SCLK --output diagnosis.json
pcb-maker inspect-routing-run <run-dir> --terminal U3:52 --access --cut \
  --output u3-52.json
pcb-maker route-kicad-board <source-dir> Remapper <out-dir> auto \
  --grid-pitch-mm 0.05 --work-budget-seconds 60 --search-threads 1
pcb-maker compare-routing-region <reference-dir> <run-dir> \
  --box-mm 134,123,144,133 --layers F.Cu,B.Cu --output u3.html
```

Illustrative report excerpt (unmeasured access fields remain explicitly unknown):

```json
{
  "schema_version": 1,
  "units": "mm",
  "input": {"board_sha256": "...", "project_sha256": "...", "binary_sha256": "..."},
  "checks": {
    "router": {"open_nets": 19, "unreached_terminals": 20},
    "native": {"status": "failed", "unconnected_pairs": 20,
               "new_errors": {"starved_thermal": 1}}
  },
  "connections": [{
    "net": "QSPI_SCLK",
    "status": "partial",
    "components": [
      {"pads": [{"ref": "U3", "number": "52", "uuid": "eae0ab9f-65b2-44d3-aba7-873c3227fba7",
                 "at": [138.9, 130.2625], "layers": ["F.Cu"]}]},
      {"pads": [{"ref": "U5", "number": "6", "uuid": "7cd0194c-2aba-437b-a016-5ca4880c9dd2",
                 "at": [136.25, 125.525], "layers": ["F.Cu"]}]}
    ],
    "rules": {"track_width_mm": 0.2, "clearance_mm": 0.2,
              "sources": ["project.net_settings.classes.Default", "project.board.design_settings.rules"]},
    "access": {"status": "unknown", "reason": "diagnostic_outline_unsupported"},
    "cause": {"status": "hypothesis", "candidates": ["tight_escape", "local_congestion", "search_order_or_budget"]},
    "reference": {"connected": true, "layers": ["F.Cu"], "track_widths_mm": [0.2], "vias": 0}
  }]
}
```

Acceptance additionally needs explicit count semantics, stable object IDs across source/result/reference, diagnostic provenance and configuration compatibility. Native connection pairs are witnesses to disconnected components, not necessarily a unique list of all isolated pads. When an access probe eventually runs, it should return attempted exact/grid access points and blocking object IDs/rules; it must not fill the unknown fields by guessing from a hot spot.

## Board evidence and plausible fixes

Source: `jfedor2/hid-remapper`, `custom-boards/v7/kicad/Remapper.kicad_pcb`, commit `51ab8b367b810d1deb0d3e487e9b7fe8c7e9c24e`. Two copper layers, 133 footprints, 596 pads, 120 router connections. Project rules are retained, not inferred. The reference has 1,052 track segments and 100 vias. Its native report has only three cosmetic `footprint_type_mismatch` findings, no relevant errors and no opens.

### Localized observations

All 20 native deep missing-connection pairs involve U3 or U103, the two `RP2040-QFN-56` footprints centered at (139.9, 133.7) and (154.5, 132.9) mm. All ten reported hot spots are on F.Cu around their bounds. This is strong localization evidence, not proof that all blockers lie inside those bounds.

| Representative open | Native endpoint coordinates, mm; all listed pads on F.Cu | Designer reference and implication |
|---|---|---|
| `QSPI_SCLK` | U3:52 (138.9, 130.2625) ↔ U5:6 (136.25, 125.525) | Seven 0.2 mm F.Cu segments, zero vias. Straight initial escape at x=138.9. Deep has no copper on the net. |
| `QSPI_SD2` | U3:54 (138.1, 130.2625) ↔ U5:3 (136.25, 128.375) | Five 0.2 mm F.Cu segments, zero vias. Deep has no copper. |
| `Net-(U3-USB_DM)` | U3:46 (141.3, 130.2625) ↔ R4:2 (141.83, 127.51) | Three 0.2 mm F.Cu segments, zero vias. Deep has no copper. |
| `CTS1RTS` | U3:38 (143.3375, 132.7) ↔ U103:35 (151.0625, 132.7) | One straight 0.2 mm F.Cu segment. Still open even in the narrow control. A custom centerline-crossing check found no foreign F.Cu segment crossing that exact straight reference line in the deep output; this does **not** test clearance halos, pads, vias or pours. |
| `+1V1` | U3:50 (139.7, 130.2625), U3:23 (140.5, 137.1375), each separated from its existing tree | Two unreached terminals on a six-terminal net. Counts alone do not identify them. |
| `+3V3`, `XIN` | U3:22 (140.1, 137.1375), U3:20 (139.3, 137.1375) | Remaining supply/crystal escapes; other branches are routed. |

Other deep partial nets are `+1V2`, `A_GPIO1`, `B_GPIO1`, `SDA`, `JACK13`, `Net-(U103-USB_DM)`, `1XOUT`, `SWD`, `TX1RX`, `1QSPI_SCLK`, `1QSPI_SD0`, `1QSPI_SD1`. The complete native pairs, UUIDs and positions are in `native-open-items.json`.

**High-confidence geometry:** U3 upper pads are 0.2 × 0.875 mm at 0.4 mm pitch. With 0.2 mm trace width and 0.2 mm clearance, a straight escape between neighboring pad extents has zero lateral margin: `0.4 - 0.1 - 0.1 - 0.2 = 0`. The reference realizes such centered escapes. U103 is the corresponding rotated footprint. This makes small access offsets consequential.

**Supported hypothesis, not proven cause:** exact pad escape construction, lattice transitions and negotiated congestion around these fine-pitch rows deserve targeted inspection. The selected deep grid is 0.075 mm with origin (98,55); some pad centers do not lie on its x lattice. But the quick run's 0.1 mm grid aligns with U3's upper-pad x coordinates and still fails the same four representative nets. Thus pitch misalignment alone cannot explain the failures. Empty `dead_pads` does not resolve this; the access/cut commands never reached their analysis.

**Separate proven native issue:** GND PTH U3:57 at (141.175,134.975), B.Cu zone UUID `7d9da784-0bc6-4ed6-b521-2c987bfbb685`, has one thermal spoke where the project requires two. Its pad UUID is `b5071759-a4d7-4769-be02-251f23cd4454`. GND is marked routed and `starved_thermals` is empty in the saved report, whose native check was skipped. This is not evidence of a completed native check returning a false negative; an unperformed check cannot establish absence of thermals. The newly filled native result makes this a thermal/plane connectivity-quality issue, distinct from the 19 open signal/power nets. No plane rewrite is justified by this single finding.

Plausible original-rule fixes to investigate are preserving exact centered escapes before lattice routing, reserving local fanout corridors, and local rip-up/order changes around U3/U103, followed by native validation. A fixed finer-lattice experiment could discriminate part of the access hypothesis, but its supported invocation was not discoverable. Repairing the identified GND thermal requires restoring a second legal spoke or rerouting nearby copper, then refilling and checking. These are proposals, not validated patches. Enlarging the board, adding layers or relaxing minimum widths is not supported as necessary by the evidence.

## Existing runs and one bounded diagnostic control

Native checks below were newly run with KiCad 10.0.7, `--refill-zones`, no `--save-board`. The saved runs and their original verdicts were left untouched.

| Result | Requested ladder work | Reported work | Grid | Router open nets / unreached terminals | New native opens | New relevant native errors |
|---|---:|---:|---:|---:|---:|---|
| Existing quick | 30 s | 32.134 s | 0.1 mm | 27 / 79 | 48 | None beyond opens |
| Existing deep | 300 s | 352.724 s | 0.075 mm | 19 / 20 | 20 | 1 starved thermal |
| Narrow-four diagnostic | 30 s | 41.351 s | 0.1 mm | 22 / 31 | 31 | 35 track-width violations |

The quick router terminal count (79) and native missing-pair count (48) are different metrics, not interchangeable validation results. The reports do not provide a mapping explaining the difference. “New native opens” means measured in this trial, not a subtraction from the original run's nonexistent native check.

Only **one new routing control** was run. Its documented `net_classes` override requested 0.15 mm for `QSPI_SCLK`, `QSPI_SD2`, `Net-(U3-USB_DM)` and `CTS1RTS`; other settings used one seed and a 30 s ladder budget. Source geometry, two layers, placement, clearances and board minimum were retained. Public descriptions compared by footprint UUID confirm identical footprint records; raw list order changes on write. Outline/layer descriptions also match. Output project adds only the diagnostic class/patterns to its net-class settings.

The first three nets became routed, each with two vias; `CTS1RTS` stayed open. This supports sensitivity to width/search state, not an isolated causal explanation: the override changes routing order/congestion, the realized work is higher, and the successful routes violate the original minimum. In particular, the control is **not an original-rule matched fix**. Its 900 s route deadline differs from the old quick run's 90 s deadline to allow a busy host; neither reported a wall cut. The old run's thread count is not established from its result JSON, so no timing or determinism comparison is claimed.

Control resources: serial execution, nice 19, `RAYON_NUM_THREADS=1`, disk-backed trial `TMPDIR`, 4 GiB address-space ceiling, start gate requiring at least 14 GiB available (10 GiB reserve plus process limit), process-group kill at 960 s. Available RAM at routing start was 32.90 GiB. It finished without timeout/cut in 71.696 s wall and 63.278 s child CPU; native check took 8.085 s wall. The outer limit bounds native/check work separately at 120 s. No existing collection process was stopped or reconfigured. The requested search-thread limit does not promise all helper/native threads are single-threaded.

The existing deep report's overall wall time was 271.822 s; its selected routing attempt was 59.874 s. This distinction is precisely why a structured attempt ledger matters.

## Friction accounting and evidence

Nine friction entries were recorded in `friction-log.json`. Per-step human/agent effort was not timed; initial subjective minute estimates were removed because they overstated elapsed time. Effort is described qualitatively. The authoritative process timings are in `commands.jsonl`: 14 captured invocations, 97.028 s summed command wall time and 106.367 s summed child CPU. These are tool execution totals, not total trial duration.

The saved command transcript contains 14 invocations: five CLI help captures, two descriptions, one connection inspection, two failed specialized diagnostics, three native DRC checks, one routing control, and the later control description is included in those two descriptions. The initial help was also issued interactively before being recaptured, and there were separate shell/JSON inspection calls. Both failed diagnostics exit 1 with exactly:

```text
error: KiCad grid adapter requires one closed Edge.Cuts outline encoded as a gr_rect or an unambiguous loop of gr_line objects
```

Custom trial-only utilities are `probe.py` (bounded command capture), `geometry.py` (KiCad geometry export), `control.py`, `sketch.py`, and `summarize.py`. A first `plot.py` attempt failed because matplotlib is absent in both interpreters; the dependency-free SVG sketch replaced it. The first geometry export also required correcting KiCad's per-layer via-width API. These workaround costs should not be attributed to the solver, but a supported endpoint/geometry query would avoid them. No reusable utility or implementation code was added outside the gitignored trial folder.

Evidence root: [`benchmarks/expansion/data/agent-trials/remapper-astra-medium/`](../../benchmarks/expansion/data/agent-trials/remapper-astra-medium/).

- `trial-request.json`: parent-recorded trial scope/model/source metadata; left intact.
- `paths.json`: absolute existing quick/deep paths; their `result.json`, `route.log`, `router-config.json`, `source/`, `reference/`, `routed/`, and `source-drc.json` remain the authoritative originals.
- `commands.jsonl`, `*.stdout`, `*.stderr`: exact argv, limits, exit codes, timing, failure text and native stdout.
- `native-quick.json`, `native-deep.json`, `native-narrow-four.json`: raw new DRC reports. CLI exit 0 means successful execution here; contents determine findings.
- `describe.stdout`, `describe-narrow-four.stdout`, `narrow-four-effective-rule-diff.json`: project-view rule values before/after. Changes from 0.2 to 0.19999999999999998 are floating representation, not a meaningful narrowed width.
- `reference-geometry.json`, `deep-geometry.json`, `native-open-items.json`: raw geometry and native witnesses used to localize failures.
- `narrow-four-config.json`, `narrow-four/`: exact control and emitted output; never a validated fix.
- `summary.json`, `sha256-manifest.json`: compact numeric outcomes and hashes of binary, input/project/config/output/evidence.
- [`u3-escape-comparison.png`](../../benchmarks/expansion/data/agent-trials/remapper-astra-medium/u3-escape-comparison.png): reference versus deep geometry sketch. Pads use rectangular extents and zones are omitted; this is orientation evidence, not a clearance proof.

Pinned binary SHA-256: `e91e8441c1b4c3c54ab04bd0b98f99a6fc44fb325fa1322f3184ed6490ac7c09`.
Original board SHA-256: `c76b92daf0e0d3bb1b96e09f2c017e0bb78540e84d58f78972658a6862d32805`.
Original project SHA-256: `f62fc72b7daffd04af809629cb4a82cd8d46041e478341e81bd88504fd05c579`.
Stripped input SHA-256: `8afa00545d4d9b1491d24095b59dc1073af910d36327a6c4ffa45fc9ca7b4dec`.

Actual command families used, with complete absolute expansions in the transcript:

```sh
$BIN describe-kicad-board "$DEEP/source" Remapper
$BIN inspect-kicad-connections "$DEEP/routed/Remapper.kicad_pcb"
$BIN inspect-kicad-terminal-access "$DEEP/routed/Remapper.kicad_pcb" QSPI_SCLK "$TRIAL/terminal-access.json"
$BIN inspect-kicad-routing-cut "$DEEP/routed/Remapper.kicad_pcb" QSPI_SCLK "$TRIAL/routing-cut.json"
kicad-cli pcb drc --format json --refill-zones --severity-all -o "$TRIAL/native-deep.json" "$DEEP/routed/Remapper.kicad_pcb"
$BIN route-kicad-board "$QUICK/source" Remapper "$TRIAL/narrow-four" "$TRIAL/narrow-four-config.json" --reference "$QUICK/reference"
$BIN describe-kicad-board "$TRIAL/narrow-four" Remapper
```

Confidence summary: high for observed counts, pad geometry, reference paths, command failures and native violations; medium for fine-pitch escape/congestion sensitivity; unresolved for exact failed search reason, legal alternative accesses, blocking clearance objects, and which original-rule algorithmic intervention will finish the board. No full routed fix is claimed.
