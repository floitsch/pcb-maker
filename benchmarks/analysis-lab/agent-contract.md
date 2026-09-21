# Blinded analysis task

You are inspecting fixed-placement two-layer routing. Find worthwhile geometric
improvements, if any. Some boards may already be good. Do not assume a number
or type of defects. Avoid trading away electrical or geometric validity.

Objective: minimize total physical route length in mm plus 2 mm per via.
Keep all pads, terminal layers, obstacles, board bounds, widths, clearance,
and connectivity fixed. Existing routes not explicitly replaced remain fixed.
Planar points use mm. A route has one layer string (`top` or `bottom`) per
segment, and changing layers at an interior point creates a through via.
Via copper exists on both layers. All obstacles are hard routing keepouts on
their listed layers. Edges of copper must meet clearance from foreign copper
and obstacles. Preserve route endpoints and their allowed pad layers.

An improvement is an explicit replacement of one or more complete routes.
Return one result per case in this JSON shape:

```json
{
  "case_id": "case-id",
  "diagnosis": "Specific reasoning, including any relevant blockers",
  "confidence": 0.8,
  "replacements": [
    {"route_id": "R", "points": [[4, 10], [36, 10]], "layers": ["top"]}
  ],
  "expected_benefit": "What should improve",
  "missing_evidence": "What query or view would most help, or none"
}
```

Use an empty replacements list when no worthwhile edit is supported. This is
an abstention, not a claim that the board is globally optimal. Submit your
initial proposals before any repair-validation feedback. Do not inspect the
generator, private directories, experiment reports, other agent outputs, or
the original project artifacts. Only use your assigned presentation files and
the action contract. No other agents may be spawned.

At each meaningful inspection step send a short observation to the parent:
what you inspected, the uncertainty you encountered, and what specific missing
evidence would resolve it. Do not narrate hidden reasoning. In your final
report distinguish visually estimated coordinates from exact supplied values.
