# Represent narrow pad passages without globally refining the grid

The legal trace-center band between two 0.37 mm-spaced pad edges is only
0.02 mm wide at 0.15 mm trace width and 0.10 mm clearance. On the retained
dual-ESP32 source, 31 of 70 such bounding-box bands contain no line of the
configured 0.05 mm grid. The [coverage inventory](../../build/routing-improvements/pad-gap-grid-alignment-2026-09-07/dual-grid-coverage.json)
binds this observation to the source board hash and actual candidate grid
origin. Its [simple rendering](../../build/routing-improvements/pad-gap-grid-alignment-2026-09-07/dual-grid-coverage.png)
marks represented bands green and missed bands red. This is a grid-coverage
observation, not proof of end-to-end routability through every band.

## Implementation

`KiCadGridRouteConfig.grid_alignment` now accepts `nearest_narrow_pad_gap`.
The default `board_origin` retains the previous grid and serialized defaults.
The alternate policy considers narrow, currently missed channels between
foreign pads of one footprint on a common copper layer. Bounding boxes and
each pad's local-clearance floor define a conservative center band.
Candidate passages must cross between terminals on opposite sides and pass
the existing continuous obstacle and board-edge checks. The policy ranks them
by estimated terminal-to-passage detour and shifts one grid axis to the selected
band's center. It preserves grid spacing, track width, clearance and search
budget; the node count cannot increase. The candidate records the footprint,
layer, band, passage, detour and offset in `grid_alignment_evidence`.

This does not represent all gaps simultaneously. A shifted grid may lose a
different passage or an edge-adjacent grid line. The new benchmark portfolio
therefore offers both the original grid and the aligned grid, with at most two
candidates and native admission before selection. The connection-insertion
driver resolves alignment against the actual board before rasterization and
skips an equivalent grid when the alignment policy makes no shift. Existing
equivalence counters record that skipped work.

## Focused native result

On the identical focused open-profile board at 0.05 mm resolution, alignment
shifts Y by 0.025 mm. The router crosses the gap with zero vias and 161 A*
expansions, versus a zero-via detour with 22,542 expansions. Native board
centerline length falls from approximately 9.106 mm to 8.025 mm. Grid dimensions
change from 258 × 178 to 258 × 177. The blocked profile returns the identical
route, origin and expansion count with either alignment setting.

All four direct-routing results pass native verification and retain previews.
The integration regression checks actual passage use, fixed grid spacing,
route-quality improvement, blocked-profile equivalence and render success.
Unit tests cover translated boards, clearance overrides, blocked and occluded
channels, already represented bands, irrelevant terminals and portfolio
equivalence. All 98 native-adapter library tests pass.

The first four-case corpus also completes with native verification. On the
three-connection dual-ESP32 open case, however, the additional candidate improves
native length by only about 0.002 mm. This is insufficient evidence to enable
the alternate policy everywhere. The policy remains explicit and the original
benchmarks remain available. Before the equivalence fix, the blocked profiles
unnecessarily searched the same grid twice; that run is retained as diagnostic
evidence in [the initial corpus](../../build/esp32-pad-gap-alignment-2026-09-07/pad-gap-summary.json).

After the equivalence fix, all four cases were rerun from their frozen empty
boards through native-gated connection progression. Every selected candidate
is identical to its initial-corpus counterpart, and final native copper and
placement hashes match. The blocked cases skip all redundant aligned entries:

| Case | Native length | Vias | Portfolio A* expansions | Equivalent entries skipped |
| --- | ---: | ---: | ---: | ---: |
| focused blocked | 9.231 mm | 0 | 23,764 | 1 |
| focused open | 8.025 mm | 0 | 22,703 | 0 |
| dual prefix 3 blocked | 93.875 mm | 0 | 330,437 | 3 |
| dual prefix 3 open | 92.380 mm | 0 | 474,679 | 0 |

Portfolio expansions include both searches when distinct grids are evaluated;
the focused aligned search alone uses 161. The existing finer-grid focused
baseline uses 0.02 mm, so its expansion counts are not a controlled phase-only
comparison. No general wall-time improvement is claimed. Freerouting was not
rerun during the deduplication check.

[Validation](../../build/routing-improvements/pad-gap-grid-alignment-2026-09-07/validation.json)
includes retained render paths for every final case. The [focused aligned
rendering](../../build/routing-improvements/pad-gap-grid-alignment-2026-09-07/deduplicated/gap-open/final-preview.png)
shows the actual native copper crossing the pad gap.

## Remaining limitations

The policy uses one conservatively modeled same-footprint pad channel per
request. It is neither an adaptive mesh nor local trace neck-down. Larger
real-board cases must establish when an extra phase is worth searching; a
small focused success is not a general routing-speed claim.

The focused result also exposed a separate quality-accounting discrepancy:
the candidate's pad-anchor return segment overlaps its preceding segment by
0.025 mm. Candidate quality reports 8.050 mm while native board centerline
union reports 8.025 mm. The board renders and passes native verification;
candidate scoring still needs to use the same union calculation for internal
branch overlaps. The lengths reported above use native board statistics.

That adjacent-backtrack omission is now [fixed in the shared graph
normalizer](2026-09-07-candidate-copper-union.md). The identical retained
candidate measures 8.025 mm and passes native verification with unchanged
occupied copper. Historical measurements above retain their original meaning.
