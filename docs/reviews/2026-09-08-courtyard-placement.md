# Native courtyard placement and the next area constraints

Follow-up: [automatic silkscreen repair](2026-09-08-silkscreen-layout.md) now
allows the full pipeline to admit both 80% and 70% area. The findings below
describe the earlier geometry/routing stage before that repair was added.

The final cold-placement run at **80% area** routes all nine nets with our
router: **362.705 mm of track, 3 vias, zero native opens**. It adds no findings
to its placement baseline. Its 18 existing silkscreen findings still prevent
area admission; the run is evidence of smaller-board routing feasibility,
not a fully admitted smaller-board result.

The known human ECC83 placement now passes the production placement validator
unchanged. The old adapter rejected four body overlaps because it used one
bounding rectangle around all footprint graphics. Native courtyards show no
body overlaps; the closest pair, P2/R1, has 0.251 mm separation. Circular socket
and mounting-hole geometry matters, and silkscreen markings are not body bounds.

P4's courtyard overhangs the board by 1.416 mm. The explicit
`benchmarks/real/ecc83-pp/placement-policy.json` defines an all-free geometric
repacking experiment: mounting holes and connectors may move, original
orientations are fixed, and all four terminal-block bodies may overhang any
edge by up to 1.5 mm. Pads stay inside with native copper edge clearance. This
policy is not a claim about the original product's mechanical requirements.

## Production changes

`layout-trace-model::Component` has optional `placement_geometry`. Absent this
field, the legacy pad-extended rectangular exclusion remains. When present:

- `body` is a circle or rectangle centered on the component origin; `size`
  must equal its local bounds. Rectangle orientation belongs to the pose.
- `clearance` is the mechanical margin outside that body. Native courtyards
  already include a mechanical margin, so this adapter adds zero.
- `board_overhang` applies only to the body. Explicit component regions
  remain strict; pads cannot inherit the body's overhang allowance.
- `pad_edge_clearance` constrains copper-pad bounds against the board outline.

The placer checks mechanical bodies and individual pad primitives separately.
Pad pairs keep copper clearance even when body clearance is zero. Circle/rect
checks include the nearest-corner axis; rotated circular bodies retain their
true board extent. Legacy components in mixed problems retain their conservative
envelopes. Pad checks currently remain conservative across layers and nets.

All 37 placement library tests pass, including circle corner space versus real
overlap, rotated bodies, retained pad clearance, strict regions, body-only
overhang and malformed policy rejection. The positive whole-board control also
passes with all 15 components fixed and zero motion.

## Experiments

The initial native-courtyard run, using the same Python random initializer as
the old probe, still fails legalization at 100%, 90% and 80% area. Geometry
fidelity does not guarantee convergence.

Switching to the production random initializer, configured for up to 32 starts,
finds a legal placement on its **first** proposal at each of those areas. This
does not establish a benefit from multiple attempts. All 15 footprints and 33
pads survive native translation with identity, net, geometry and orientation
inventories intact. Native verification then reports:

| Area | Total design findings | Copper edge findings | Other findings |
| --- | ---: | ---: | --- |
| 100% | 18 | 2 | Silkscreen |
| 90% | 16 | 2 | Silkscreen |
| 80% | 21 | 3 | Silkscreen |

The pad-boundary projector originally permitted copper to touch the outline.
That violated the source project's 0.01 mm edge rule. The adapter now reads
the native setting into `pad_edge_clearance`, and the production projector
enforces it independently of connector overhang.

Silkscreen follows component motion and can overlap nearby parts or cross the
outline. It needs its own layout/cleanup step. The runner may route an otherwise
physical-clean placement with annotation findings to diagnose routability, but
**area admission still requires zero placement findings and full routing**.
Existing severities remain unchanged. No smaller fully admitted board is
established by the above placement results.

## Final 80% routing result

After the edge-clearance correction, the human control still passes unchanged
and the production random initializer again finds a legal first proposal.
The final native outline area is 1930.963866 mm², versus 2413.704850 mm² for
the original. Native pad-edge findings fall to zero; all 18 remaining design
findings are silkscreen-related. The complete comparison process exits zero.

| Router | Native opens | Track | Vias | Route admission against placement baseline |
| --- | ---: | ---: | ---: | --- |
| pcb-maker | 0 | 362.705 mm | 3 | Pass |
| Freerouting | 0 | 352.394 mm | 2 | Fail: annotation-position differences |

Both receive matching basic routing rules and fixed placement. The DSN class
and net/pin audit passes on this second board family too. Freerouting's 18
native findings have the same type, severity, description and item identities
as the source, but 17 reported item positions shift by up to 0.000030 mm after
import. The strict baseline matcher therefore counts 15 findings as introduced.
`annotation-roundtrip.json` records this diagnosis. It is not a complete
geometry-equivalence proof, and admission has not been relaxed.

Our final native inventory matches the original 15 footprints and 33 pads,
including raw nets, pad geometry, relative positions and fixed orientations.
All 31 pinned input files remain unchanged. The frozen final executable is
`2b7cef4f2a5686d99feba6e85428cb95fcdacc8dfe8dad20c6a6ecb85ab84e88`.

The final planted suite contains the human positive control and five faults:
socket/body collision, forbidden connector overhang, extra body clearance,
an outside pad despite permitted body overhang, and an external pad entering
the socket. Both the independent checker and the production validator produce
all six expected outcomes. Every case has a rendering including pad bounds.

This is a feasibility point, not an optimized placement frontier. The all-free
harmonic block has no positional anchors, so its existing policy deliberately
skips harmonic attraction and uses the random initializer plus legalization.
Selecting and improving feasible placements by whole-board routing quality
remains ahead, along with silkscreen layout and the roundtrip admission audit.

## Analysis tools and evidence

`courtyard_geometry.py` extracts supported native courtyard shapes, reports
signed pair gaps and exact overhang distances, and renders the geometry.
Unsupported/missing courtyard forms are rejected. `area_probe.py` independently
checks those bodies plus conservative pad rectangles, then invokes the
production placement gate on the human positive control.

`audit_native_placement.py` checks native pad identities, shapes, sizes, drills,
layers, raw nets and relative positions; footprint identities, values and
orientations; exact requested translations; the new rectangular outline;
zero inherited copper; and unchanged project bytes. This is geometry evidence,
not a datasheet or electrical design review.

- [Original native courtyard rendering](../../build/ecc83-courtyard-control-2026-09-08/preview.svg)
- [Explicit-policy independent control](../../build/ecc83-courtyard-control-2026-09-08/policy-control.json)
- [Single-initializer failures](../../build/ecc83-courtyard-area-2026-09-08/index.html)
- [Production random starts and native findings](../../build/ecc83-courtyard-multistart-2026-09-08/index.html)
- [Final smaller-board routing and native previews](../../build/ecc83-courtyard-edge-routing-2026-09-08/index.html)
- [Final validation evidence](../../build/ecc83-courtyard-edge-routing-2026-09-08/validation.json)
- [Planted fault controls](../../build/ecc83-courtyard-planted-controls-final-2026-09-08/index.html)

Every placement experiment writes a rendering; native verification also writes
its own preview. Images have been inspected, but browser playback is untested.
