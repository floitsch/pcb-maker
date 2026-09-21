# Board analysis laboratory

This benchmark measures whether a fresh agent can discover and specify a valid
PCB routing improvement, and which presentation or inspection tool helps it.
It includes synthetic two-layer geometry and native ECC83 and ESP32-C3 boards.
Synthetic acceptance and native KiCad admission use separate evaluators.
Results and limitations are recorded in
[`docs/experiments/board-analysis-lab.md`](../../docs/experiments/board-analysis-lab.md).
The [analyst playbook](analyst-playbook.md) describes the current working interface.

## Protocol

- Fixed pads, rules, and connectivity. Score physical route length plus 2 mm
  per via, after exact validity. Routes in the initial fixtures do not share
  collinear copper, so summed branch length is physical length.
- Public fixtures carry neutral IDs and no mutation metadata. The private
  answer key contains one validated repair, not an assertion of optimality.
- Fresh agents receive no parent conversation. They must not inspect private
  fixtures, generator source, other agents' reports, or experiment notes.
  This is protocol isolation on a shared filesystem, not enforced access isolation.
- Agents submit their initial diagnosis before receiving exact repair feedback.
  Subsequent repair attempts are separately counted.
- Controls contain no *planted* improvement; any valid improvement found on a
  control still earns credit. Absence of a discovered improvement does not prove
  global optimality.
- Record localization, explanation, actionable edits, exact validity, quality
  delta, elapsed wall time, visible tool calls, and requests for missing evidence.
  Hidden image reasoning is not observable, and self-reports are qualitative.
- Development feedback may inform tools. Reused cases then become development
  cases; they cannot establish generalization. Use fresh transformed and held-out
  mechanisms for subsequent checks.

## Presentations

1. Compact numerical geometry alone.
2. Simple layer renderings plus the common rules/action contract.
3. Both representations with matching IDs.
4. Interactive net/region views and topology summaries.
5. The strongest inspection interface plus bounded counterfactual checks.

Initial agents use the same model and inherited reasoning configuration. Small
pilot differences are descriptive, not statistical evidence of a winner.
Presentation size and rendering scale are recorded; elapsed time includes tool
latency. Token accounting is reported only if the runner exposes actual usage.

## Inspection

`inspect_board.py BOARD summary` reports lengths and layer transitions.
`inspect_board.py BOARD net --net N` adds pads and foreign context.
`inspect_board.py BOARD render --output IMAGE.png` renders aligned layers.
`--net N`, `--bounds X0 Y0 X1 Y1`, and `--overlay` select alternate views.

`anchors` returns exact endpoint IDs, coordinates, pad layers, and rules.
`probe --from X Y --to X Y --layer top --net N` returns blockers and nearest
clearance margins for a proposed planar segment. This diagnostic query does
not certify connectivity, vias, board boundaries, or a complete repair.
`probe-via --at X Y --net N` instead checks through-via copper on both layers.
Both probes accept `--proposal FILE` for explicit simultaneous-edit context;
they remain diagnostics and do not invoke the exact validator.
`compile --proposal FILE` resolves `"start"`, `"end"`, `{"pad":"P1"}`, or
`{"point_index":1}` references in proposed points. These references preserve
exact coordinates without requiring the image reader to recover them from pixels.

The renderer uses Pillow and a font selected through fontconfig. It renders
geometry directly, without KiCad, and does not rank suspicious objects or read
answer keys. Gray objects are hard routing obstacles, not component decoration.

## Reproduction

From the repository root, with `target/release/pcb-maker` available:

```
python3 benchmarks/analysis-lab/generate.py generate
python3 benchmarks/analysis-lab/generate_harder.py
python3 benchmarks/analysis-lab/transform_suite.py
python3 benchmarks/analysis-lab/generate_relocation.py
python3 benchmarks/analysis-lab/generate_via_windows.py
python3 benchmarks/analysis-lab/score.py score --public PUBLIC_DIR --answers ANSWERS.json --output SCORED.json
python3 benchmarks/analysis-lab/summarize.py
python3 -m unittest discover -s benchmarks/analysis-lab -p 'test_*.py'
```

The summary command creates `build/analysis-lab/scoreboard.{json,md}` from retained
submissions. It keeps initial and cumulative recovery rows separate and excludes
fixture witnesses and diagnostic controls; it does not pool reused cases as
independent observations.

Native tools require KiCad's `pcbnew` Python module and `kicad-cli`; rendering
also requires Pillow and fontconfig. Existing native experiments used KiCad
10.0.6. Generate a copied, mutated fixture with:

```
python3 benchmarks/analysis-lab/native_fixture.py generate --source SOURCE.kicad_pcb --output FIXTURE_DIR --case-id n01
python3 benchmarks/analysis-lab/native_fixture.py evaluate FIXTURE_DIR/public/n01.kicad_pcb FIXTURE_DIR/public/n01.json PROPOSAL.json --output FRESH_RESULT_DIR
```

The native evaluator preserves board/project provenance and compares native
finding multisets, connectivity, and physical centerline union cost. A lower
score with a new finding is rejected. Its supported geometry is deliberately
limited to boards without copper zones, straight tracks, and supported pad
shapes. Current trials use two copper layers. Non-copper rule areas allowing
tracks/vias are retained as metadata, and zero-delta trapezoid tags are exported
as exact rectangles with their source tags preserved. It does not measure
return-path quality or prove production suitability.

See [native-agent-contract.md](native-agent-contract.md) for exact object and
region queries and [native-graph-tools.md](native-graph-tools.md) for conservative
connectivity graphs, explicit edge materialization, and named copper-clearance
probes. Agents may use these tools only when their experimental arm permits it.

## Extension candidates

Redundant layer excursions; necessary-via lookalikes; avoidable detours;
obstacle-corner tightening; shared-trunk duplication; poor tree attachment;
redundant junctions; relocatable vias; route-order traps; local clearance
violations; length/skew repair; congestion relieved by moving or rotating a
component; multi-net rip-up; and ground-plane/stitching quality.

Separate discovery difficulty from repair difficulty. Add clutter, narrower
clearance margins, hidden cross-layer dependencies, interacting edits, and
whole-board navigation independently. Compare against simple deterministic
shortcut/layer-excursion heuristics before attributing success to model reasoning.
