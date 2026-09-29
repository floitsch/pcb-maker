# The topological engine (TopoR's algorithm)

`crates/pcb-topo` is a second routing engine, built on TopoR's published
algorithm (Luzin & Polubasov; the research is in
[reviews/2026-09-29-topor.md](reviews/2026-09-29-topor.md)). It works in two
modes:

- **Route with it.** Pass `{"engine": "topological"}` to
  `route-kicad-board`. The board is routed topologically on a triangulation,
  layers are assigned afterwards, and the tracks are pulled tight into
  tangents and arcs, at any angle.
- **Tighten.** Pass `{"tighten": true}` and the lattice router routes as
  usual. Its copper is then read as a topology and pulled tight wherever the
  result stays legal. This gives TopoR's look (any angle, arcs, no
  staircases) with the lattice router's completion. A piece that would
  become illegal keeps its original copper, so the result is never worse.

Both only ever emit copper that the exact verifier accepts. An engine
failure leaves connections open; it never produces an illegal board.

## Using it

```sh
echo '{"engine": "topological"}' > topo.json
pcb-maker route-kicad-board <project dir> <board id> <out dir> topo.json

echo '{"tighten": true}' > tight.json      # lattice router, then tightened
pcb-maker route-kicad-board <project dir> <board id> <out dir> tight.json
```

The engine's knobs sit under `"topological"`, and every one of them is
optional:

| key | default | meaning |
| --- | --- | --- |
| `crossing` | 2 | mm one crossing with another net costs while routing |
| `via` | 8 | mm a via costs while routing and assigning layers |
| `improve_via` | 25 | mm a via costs while improving a legal board |
| `improve_passes` | 4 | passes of the optimization over all wires |
| `spacing` | 2 | spacing of via sites in open areas (mm) |
| `seconds` | 300 | time budget |
| `layered_start` | false | route the first time layer by layer, not TopoR's way |

`topological_rounds` (default 30) sets the repair rounds. Pour nets are
routed as tracks, since the engine does not connect to pours yet.

## How it works

The board is held in four separate representations, one per stage:

1. **Triangulation** (`mesh.rs`). One constrained Delaunay triangulation
   covers every obstacle and the outline, and all layers share it, as in
   TopoR.
   - Round pads are 16-gons.
   - Long sides are split at 0.8 mm, so that every narrow gap is an edge.
   - **Via sites** are free vertices where a via of every class fits. They
     sit on a lattice of the via pitch near obstacles and sparser in the
     open. Two vias can never collide, and each site holds at most one
     wire.
   - Each edge knows its *cut*, the free distance across it per layer,
     measured to round obstacles' true shapes.
2. **Topology** (`topo.rs`). A wire is a sequence of portals:
   - an edge it crosses, together with its place in that edge's order of
     crossings (one order for all layers);
   - or a via site it passes through.

   After layer assignment, the wire also has a layer in every face it
   passes. A layer change happens only at a site, and that is a via.

   Room on an edge is its cut, less the clearances its ends keep off. Other
   wires' vias count at the ends. n tracks take n × (width + clearance)
   less one clearance.

   The search is an A* over (portal, gap, layer). It has three modes:
   - **Planar.** TopoR's first routing: layers are ignored, crossings with
     other nets cost `crossing`, and room is summed over the layers.
   - **Layered.** The search carries a layer and may not cross a wire on
     that layer. It pays for vias and for overflow per layer.
   - **Strict.** Layered, and no edge may overflow.

   An edge that cannot hold one track is never crossed. A via is placed
   only where the wires already passing it keep their room.
3. **Layer assignment** (`layers.rs`). This is TopoR's "расслоение".
   - Wires that cross in a face must differ there, a terminal's face takes
     a layer of its pad, and each layer's room per edge is shared.
   - Each wire's best layers, given all the others, come exactly from
     dynamic programming over its faces. Passes over all wires in random
     order repeat until nothing improves.
   - On two layers, *clusters* also flip together. A cluster is the set of
     stretches (between via sites) linked by crossings. Flipping all of
     them keeps every crossing apart, and simulated annealing over the
     clusters minimizes vias. This is the practical version of TopoR's
     exact 2-layer max-cut.
4. **Geometry** (`realize.rs`, `taut.rs`). Each wire is cut at its vias
   into pieces, one per layer. On each layer, the obstacles and vias are
   triangulated again, with round shapes as points that have a radius. Each
   piece's embedding (the polyline through its places) is traced through
   that triangulation to give its channel.
   - **Taut path.** The piece is pulled tight in that channel. The funnel
     algorithm on portals shrunk by the radii guesses which discs it
     wraps. Exact distances then add discs the path comes too close to and
     drop discs it turns away from. The result is tangent segments and
     arcs.
   - **Nesting.** A piece keeps its room from the pieces of other nets
     that pass a vertex on its inside. Those count with their wrap radius,
     or with their actual distance when this piece wraps the vertex too.
   - **Running along.** Pieces that still come too close run along each
     other. The outer one goes around the inner one's bends, grown by the
     room between them.
   - **Exact obstacles.** Exact distances to obstacle shapes add the
     corners of any obstacle still too close.
   - **Pad ends.** A track ends where it enters its pad, half a width
     inside, or at the first legal point inside the pad.
   - **Arcs.** They are emitted as polylines that circumscribe them and
     stray at most 0.5 µm outside.

### The driver (`lib.rs`)

1. **Instant routing.** Every connection is routed in planar mode, widest
   nets first, then the shortest connections. This is TopoR's "100 %
   instantly", rules violated or not: Interf-U takes 0.5 s. With
   `layered_start`, the wires are routed layer by layer instead.
2. **Layer assignment.**
3. **Repair.** The rounds alternate three steps:
   - **Negotiation in the model.** This is PathFinder. The wires that
     overflow an edge, cross on one layer, or are unrouted are rerouted.
     Each iteration raises the price of overflow and adds to the history
     of the overflowed edges, until the model is clean.
   - **Geometry and the exact verifier.**
   - **Learning.** Where the geometry did not fit, the model had room it
     did not have. The missing millimetres are taken off the edges the
     wire crosses there, and the wires in trouble are rerouted.

   The best state is kept.
4. **Improvement.** This is TopoR's optimization. Every wire in turn is
   ripped up and rerouted in strict mode with vias priced at
   `improve_via`, and the new route is kept if it is cheaper. The layers
   are then reassigned, the result is repaired, and the pass is kept only
   if the board is at least as good.
5. **Output.** Wires still in trouble are dropped until the verifier is
   clean. They are reported as open.

`tighten.rs` reuses step 4 of the representations (the geometry). Each
net's copper becomes chains between fixed points: pad copper, vias, branch
points and ends. Every chain is pulled tight, and a chain the verifier
rejects keeps its original segments.

## Where it stands

The test set is copies of the corpus boards with every track, via and
copper zone removed (`strip-kicad-copper`). Each board is routed on its
designer's placement. Every result is checked by KiCad's DRC: 0 errors and
0 unconnected items, unless a cell says otherwise. The lattice router's
lengths run from pad centre to pad centre; the topological engine's tracks
stop at pad edges.

| board | lattice | lattice + tighten | topological |
| --- | --- | --- | --- |
| ECC83 (9 nets) | 9/9, 0 vias, 249 mm | 9/9, 0 vias, 241 mm | 9/9, 0 vias, 199 mm, 0.02 s |
| complex hierarchy (50) | 50/50, 0 vias, 1330 mm | 50/50, 0 vias, 1286 mm | 50/50, 1 via, 1120 mm, 0.6 s |
| PIC programmer (34) | 34/34, 1 via, 1911 mm | 34/34, 1 via, 1841 mm | 34/34, 1 via, 1682 mm, 1.1 s |
| Interf-U (110) | 110/110, 24 vias, 4637 mm | 110/110, 24 vias, 4478 mm | 110/110, 57 vias, 4398 mm, 73 s |

On the wider corpus (same stripping, 300 s budget), the topological
engine falls well behind the lattice router:

| board | lattice | topological |
| --- | --- | --- |
| Olimex ESP32-C3 (2 layers) | 34/34, 44 vias | 34/34, 71 vias, 244 s |
| DUT C3 (2 layers, pours) | 42/42, 22 vias | 41/42, 36 vias |
| multichannel (2 layers) | 79/79, 20 vias | 78/79, 55 vias |
| StickHub (2 layers, dense) | 45/45, 45 vias | 17/45 |
| ColdFire (4 layers) | 207/209 | 116/209 |
| video (4 layers) | 362/371 | 145/371 |
| fence (2 layers) | — | 2/101 |

The errors KiCad reports on Olimex (fiducials too close to mounting holes)
are the design's own.

**Verdict (2026-09-30).** The engine is TopoR's algorithm, and it works:
- on simple two-layer boards, it matches the lattice router on completion
  and beats it on length;
- on dense and four-layer boards, it does not come close;
- it has more vias everywhere, and on large boards its search is slow.

TopoR's own advantage lies in its geometry, and `tighten` gives us that
on top of the lattice router's completion. The engine stays as an opt-in
option and is not developed further. The lattice router is the main
engine.

## Differences from TopoR, and what is missing

- **Vias.** TopoR places them "somewhere on the stretch between two
  crossings" and nudges them later; its own manual admits violations can
  go unnoticed. Here, vias stand on sampled sites, so they are always
  legal. The cost is that a via is possible only where a site is.
- **Legality.** TopoR's "instant 100 %" is a state with violations, and
  its optimization then works them down. Here, the model negotiates, the
  exact verifier judges, and only legal copper is emitted.
- **Arcs** are polylines. KiCad has native arcs, but `NetRoute` and the
  verifier do not carry them yet.
- **Not done yet**, in rough order of value:
  - Steiner connections (a wire joining its net's existing copper, not only
    pad to pad);
  - pours (connecting to planes);
  - the engine inside `layout-kicad-board`, with moves that keep topology;
  - via nudging;
  - TopoR's archive of (length, vias) trade-offs;
  - differential pairs and length matching;
  - more than two layers is supported (Viterbi per wire), but untuned.

## Debugging

| variable | effect |
| --- | --- |
| `PCB_TOPO_SVG=<dir>` | an SVG per stage: the triangulation with the wires through their places, coloured by layer; the copper with the verifier's violations as rings |
| `PCB_TOPO_DEBUG=1` | failures per round: a summary by kind, then the first pieces without geometry (what squeezed them, where) and the first violations |
| `PCB_TOPO_TRACE=<wire>` | how that wire's radii grow (nesting, running along) |
| `PCB_TOPO_TAUT=1` | the disc sequences of a taut path that does not settle |

Unit tests for the geometry are in `taut.rs` (`cargo test -p pcb-topo`).
