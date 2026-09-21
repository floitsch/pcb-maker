# Native graph and clearance inspection tools

These tools answer geometry questions and express your decisions. They do not
choose which routing to keep or prove native acceptance. Read their output
caveats before using the graph as a connectivity model.

```
python3 benchmarks/analysis-lab/native_graph.py BOARD summary
python3 benchmarks/analysis-lab/native_graph.py BOARD graph --net N1 --output GRAPH.json
python3 benchmarks/analysis-lab/native_graph.py BOARD materialize --net N1 --selection SELECTION.json --output PROPOSAL.json
python3 benchmarks/analysis-lab/native_probe.py BOARD PROPOSAL.json
python3 benchmarks/analysis-lab/native_probe.py BOARD --batch CANDIDATES.json --compact
python3 benchmarks/analysis-lab/native_chains.py BOARD --net N1 --output CHAINS.json
```

The graph splits existing tracks at exact endpoint and pad/via-center contacts,
deduplicates overlapping centerlines, and reports source-object coverage,
terminal nodes, fixed plated-pad bridges, via edges, and costs. It conservatively
omits some physical contacts. Summary reports coverage and components so that
incomplete models are visible. It cannot discover new geometric shortcuts.

Choose edges explicitly in `{"net":"N1", "selected_edges":["e-..."],
"graph_sha256":"value from graph"}`. Materialization preserves fixed pad bridges,
checks modeled terminal connectivity and complete via groups, and writes the
selected physical segments once. It replaces that net's original track objects.
Combining proposals for disjoint nets is permitted.

All three graph commands optionally accept `--pad-contacts`. This adds fixed
zero-cost contacts from existing same-layer graph nodes strictly inside
undrilled SMD pad copper to that pad's terminal. It handles the supported pad
shapes, rotation, and offset, creates no track geometry, and labels this evidence
as an inference. Use the same flag for graph extraction and materialization.
Drilled pads, boundary-only touches, and finite-width track contacts remain
outside this added inference. Check coverage before relying on any graph.

For native direct-contact evidence, create a packet from the matching PCB and
exported geometry, then use it consistently for graph queries and materialization:

```
python3 benchmarks/analysis-lab/native_contacts.py BOARD.kicad_pcb GEOMETRY.json --output CONTACTS.json
python3 benchmarks/analysis-lab/native_graph.py GEOMETRY.json summary --native-contacts CONTACTS.json
python3 benchmarks/analysis-lab/native_chains.py GEOMETRY.json --net N1 --native-contacts CONTACTS.json --output CHAINS.json
```

`--native-contacts` and `--pad-contacts` are mutually exclusive. The native packet
binds direct `GetConnectedPads(track)` evidence and endpoint pad hit-tests to
board/project and geometry hashes. Identical pad UUID aliases are explicit;
ambiguous aliases are rejected. Native contacts add fixed graph edges without
emitting copper. Hit-tests do not establish drill-subtracted copper occupancy,
and arbitrary subsequent edits still require native DRC.

The probe reports named foreign-copper clearance blockers for added segments,
accounting for simultaneous removals. It does not check connectivity, dangling
tracks, holes, board edges, or manufacturing rules. Native validation remains
necessary after submission. Do not interpret a clear probe as acceptance.

New native packets export resolved per-pad, per-layer own clearance, including
pad/footprint overrides. Probe witnesses state the rule basis. Older packets
without those fields retain a labeled netclass fallback. This does not resolve
arbitrary custom object-pair rules or general hole clearances.

Batch input is `[{"id":"candidate-1","proposal":{...}}, ...]`, with unique
string IDs. Each candidate has its own simultaneous removals/additions, and
candidates are checked independently in input order. The tool does not invent,
rank, or select candidates. `--compact` returns blockers and minimum clearance
margin per candidate; omit it for full per-track nearest-object evidence.

`native_chains.py` lists maximal unbranched same-layer, same-width paths from
the factual graph, splitting at pad/via contacts and branches. It accepts
`--pad-contacts` too. Output contains ordered points, edge IDs, path length,
endpoint distance, and exact source coverage. Closed cycles are labeled.
This is an inventory, not a set of proposed shortcuts. Source tracks can cover
more than one chain: partially covered source IDs must not be blindly deleted.
If you choose new chords, you remain responsible for retaining all required
remaining copper and submitting a coherent simultaneous proposal.
