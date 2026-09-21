# Board analysis: current working interface

This workflow is based on the small development trials recorded in
[the experiment report](../../docs/experiments/board-analysis-lab.md). It is a
working default, not a claim that one presentation always wins.

1. Establish the objective and acceptance rules. Length and via count alone
   do not describe return-path quality, timing, or manufacturing suitability.
   Identify fixed geometry and protected routing before proposing changes.
2. Start with an overview and a compact index. For small boards, numerical
   geometry alone has been sufficient and often faster. For dense boards,
   inspect selected nets or regions and keep IDs consistent across views.
3. Ask topology questions with the physical-edge graph. Inspect terminal
   coverage before reasoning about connectivity; missing modeled contacts are
   not proof of an open circuit. Prefer a matching native direct-contact packet
   when available; interior SMD pad inference is another explicit contact mode.
4. Ask geometry questions with chain inventories, exact object coordinates,
   and clearance probes. Generate your own explicit candidate edits and batch
   independent questions. Include simultaneous removals when evaluating edits
   that depend on moving other routes.
5. For vias, inspect both layers and the via diameter. A trace fitting through
   a gap does not imply that a via fits. Use a zoomed view or via-center space
   map for discovery, and an exact via probe for a clearance witness. Endpoint
   references and anchor metadata avoid recovering coordinates or order from
   pixels.
6. Materialize selected physical graph edges explicitly. Source track objects
   may overlap or extend beyond the selected chain; source IDs are not blanket
   deletion instructions. Preserve required remaining copper.
7. Admit the complete simultaneous proposal through the exact evaluator. A
   clear copper probe does not establish connectivity, hole clearance, or
   freedom from new dangling-object findings. Keep first submissions and
   recovery attempts separate, and use mapped feedback to diagnose failures.

The tools deliberately avoid selecting improvements. Graph extraction, source
coverage, coordinate references, image rendering, and clearance calculation
remove bookkeeping so the agent can spend effort choosing and testing changes.

For a native board, see [the action contract](native-agent-contract.md) and
[graph/chain/probe commands](native-graph-tools.md). For synthetic fixtures,
see [the inspection interface](agent-tools.md). Experimental arms may restrict
these tools to measure which evidence is useful.
