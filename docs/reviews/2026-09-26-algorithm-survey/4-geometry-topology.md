# Geometry and topology ideas for the PCB placer/router

Scope: this is a literature and ideas report only; no repository code was read. I checked the key references with web searches. Anything I could not confirm is marked (?). Ratings are **V** (value: H/M/L) and **E** (effort: S/M/L).

**The main idea.** Split the router into three layers:
- **Topology layer.** It decides which side of each obstacle a wire passes, which layer it uses, and which pour faces survive. It works on a constrained Delaunay triangulation (CDT) with exact cross-section capacities. This is where feasibility can be decided, and even proved.
- **Geometry layer.** It turns the topology into exact track shapes: shortest paths within the chosen homotopy class, with arcs and exact clearances.
- **Lattice.** It is kept only as a local detail or repair engine inside the corridors (sleeves) the topology layer chooses.

Most of the pain points come from asking the lattice to make topological decisions (pour shredding, stalls around 85–90 %, runtime) or geometric ones (the 0.01 mm slack under BGAs).

---

## A. Routing substrate and topology

### 1. CDT cross-section capacity graph as the global router
- **References:**
  - Dai, Dayan, Staepelaere, "Topological routing in SURF: generating a rubber-band sketch", DAC 1991.
  - Staepelaere et al., "SURF: rubber-band routing system for multichip modules", IEEE D&T 1993.
  - Yu, Darnauer, Dai, "Interchangeable pin routing with application to package layout", ICCAD 1996. This does min-cost flow on a triangulated routing network instead of a grid, handles multiple layers, and supports any-angle, octilinear or rectilinear wiring.
  - Seong, Yang, Han, "Topology for substrate routing in semiconductor package design", arXiv:2105.07892 (2021).
- **How it maps onto a PCB.**
  - Build one CDT per copper layer. Vertices are pad and courtyard polygon corners, board-outline and keepout corners, and existing vias. Constrained edges are obstacle boundaries.
  - Every non-constrained CDT edge e = (p, q) is a cut. Its exact capacity is the largest k satisfying `d(p,q) ≥ cl(p,t1) + w1 + cl(t1,t2) + … + wk + cl(tk,q)`.
  - With mixed net classes this depends on the order of the wires. For certificates, use the upper bound given by the smallest class. For the router's pricing, use the actual multiset of classes crossing the edge.
  - Vias live in triangle interiors, as Steiner points. They use up the capacity of every layer's triangle that contains them.
  - Run PathFinder over the dual graph (triangle to triangle and layer to layer via sites). Edge costs are length plus a congestion price, the same scheme you run now.
  - The result is an assignment of each net to a sequence of triangles, which is exactly a homotopy class. This replaces the current coarse tile corridors.
- **Pain points:** 2, 5, 7, and it builds directly on your "promising" prototype of corridor assignment with width capacities.
- **Why it beats the lattice.**
  - Capacities are exact real distances, not counts of lattice nodes. A 0.75 mm BGA channel shows capacity 1 or 2 exactly, whatever the lattice pitch.
  - Each layer has O(pads) triangles (about 10⁴) instead of about 10⁷ lattice nodes.
  - Rip-up and reroute at this level is roughly 1000× cheaper.
- **Hybrid.** Keep the lattice A*, but restrict it to the corridor (the chain of triangles) of the chosen homotopy class, dilated by one triangle.
- **Main risk.**
  - CDT edges are not every critical cut. The Leiserson–Maley theory (idea 3) needs cuts between all pairs of features that can see each other across free space.
  - Delaunay edges contain the nearest-neighbour pairs and in practice catch most binding cuts. Long, skinny channels can still hide a binding cut. Mitigation: add cuts between vertices whose Voronoi regions are adjacent across free space, or refine the CDT.
- **First experiment.** Build a CDT on one BGA plus its neighbourhood using the Rust `spade` crate. Compute edge capacities and run min-cost flow for escape (idea 9). Compare the predicted routable-net count with what the lattice router actually achieves.
- **Rating:** V: H, E: M.

### 2. Rubber-band topological routing (SURF / TopoR style) as the core model
- **References:**
  - Dayan, "Rubber-band based topological router", PhD thesis, UC Santa Cruz, 1997.
  - Dai et al., DAC 1991 (as in idea 1).
  - TopoR (Eremex), a commercial any-angle topological router with arcs.
  - Schrijver, "Disjoint homotopic paths and trees in a planar graph", Discrete & Computational Geometry 1991. It shows fixed-homotopy disjoint paths in a planar graph are polynomial: a cut condition plus a parity condition decides them.
- **How it maps.**
  - The state is a rubber-band sketch: each net is a sequence of passes around obstacle vertices (left or right) per layer, plus via sites.
  - Moves are local topological changes: flip the side of an obstacle, move a via, change the order of two wires at a vertex.
  - Feasibility is checked with cut tests (idea 3). Geometry comes only at the end (idea 4).
  - Multi-layer: treat the space as a 2.5D complex, with the per-layer planes glued together at via sites. A homotopy class then includes the choice of vias.
- **Pain points:**
  - 1 and 7, because topology decides which regions are enclosed.
  - 6, because arcs and any-angle come natively.
  - 5.
- **Why beat or complement the lattice.**
  - Schrijver's result justifies the two-phase design. If the cut conditions hold for a chosen homotopy, a disjoint realization exists, so confining the detail router to the corridor loses no feasibility.
  - It makes "negotiate over homotopy classes" possible instead of "negotiate over nodes".
- **Main risk.**
  - Engineering: robust incremental sketch updates are hard.
  - SURF needed years of work.
  - Multi-terminal nets are Steiner trees, and homotopy classes of trees need care (Schrijver handles trees).
- **First experiment.** A 2-layer board of about 30 nets. Take the lattice router's output, extract its sketch (the triangle sequence per net), run cut checks, then apply "flip" local search to reduce the maximum cut overflow. Measure whether the lattice can then realize the improved sketch.
- **Rating:** V: H, E: L.

### 3. Exact single-layer routability via cut conditions, and infeasibility certificates
- **References:**
  - Leiserson & Maley, "Algorithms for routing and testing routability of planar VLSI layouts", STOC 1985.
  - Maley, "Testing homotopic routability under polygonal wiring rules", Algorithmica 15 (1996).
  - Cole & Siegel, "River routing every which way, but loose", FOCS 1984.
  - Gao, Jerrum, Kaufmann, Mehlhorn, Rülling, Storb, "On continuous homotopic one layer routing", SoCG / CG'88 (LNCS 333).
  - Multi-layer relaxation via LP duality: the "Japanese theorem" (Iri 1971; Onaga & Kakusho 1971) (?); Okamura & Seymour, "Multicommodity flows in planar graphs", JCTB 1981.
- **How it maps.**
  - **Single layer, homotopy fixed.** Routable if and only if for every pair of obstacle features (p, q), the total width plus spacing of wires whose homotopy class forces them across segment pq is at most d(p, q). The paper's wiring rules are Euclidean or polygonal. KiCad's disc clearance fits the Euclidean model, but mixed classes need the per-order bound.
    - This is an exact yes/no test of whether a passage fits, given the topology.
  - **Multi-layer.** Solve the fractional multicommodity-flow LP on the CDT dual graph with upper-bound capacities (per layer, plus via-site capacity).
    - If the LP is infeasible, its dual gives edge lengths ℓ with Σₖ dₖ·dist_ℓ(sₖ, tₖ) > Σₑ cₑℓₑ.
    - That is a valid proof that no integral routing exists in this placement: the capacities over-approximate the truth, and fractional infeasibility implies integral infeasibility.
    - Since the dual prices give each net's marginal cost, use them to decide which net yields: the Lagrangian choice is the net with the largest dₖ·dist_ℓ divided by its priority.
- **Pain points:** 7, and 4 (placer feedback).
- **Why beat the lattice.** A lattice cannot prove anything; its failures mix resolution effects with real infeasibility. Cut tests are cheap: O(n log n) over CDT edges.
- **Main risk.**
  - Via-site placement freedom makes the multi-layer LP loose, so certificates will be weak on 4-layer boards.
  - Fixed-homotopy tests only prove "infeasible in this topology".
- **First experiment.** On the 85–90 %-stalled 2-layer boards, compute CDT cut overflows of the final lattice state plus the LP relaxation. Report how many unrouted nets have a certified-overfull cut, versus nets that failed for search reasons.
- **Rating:** V: H, E: S–M.

### 4. Geometric realization: shortest homotopic fat paths, tangent graph, funnel, arcs
- **References:**
  - Hershberger & Snoeyink, "Computing minimum length paths of a given homotopy class", CGTA 4 (1994). It uses the universal cover and also covers paths restricted to given orientations, which is directly the 45° case.
  - Efrat, Kobourov, Lubiw, "Computing homotopic shortest paths efficiently", CGTA 35 (2006).
  - Bespamyatnikh, "Computing homotopic shortest paths in the plane", J. Algorithms 2003.
  - Duncan, Efrat, Kobourov, Wenk, "Drawing with fat edges", IJFCS 17(5) 2006. It covers homotopic routing of thick edges with order preserved.
  - Kallmann, "Dynamic and robust local clearance triangulations", ACM TOG 33(5) 2014.
  - Lee & Preparata (1984): the funnel algorithm.
- **How it maps.**
  - Given the triangle sleeve for a net (idea 1 or the lattice path), inflate each obstacle by the Minkowski sum with a disc of radius cl + w/2. Pad polygons become rounded polygons.
  - The shortest homotopic path is then tangent segments plus arcs of exactly that radius around corners. The track centreline hugs obstacles at exactly the clearance, so the result is correct by construction and has native KiCad arcs.
  - For k wires through one channel, process them inside-out in their order around each vertex. Each realized wire (inflated) becomes an obstacle for the next. This is SURF's successive rubber-band, with fat edges as in Duncan et al.
  - For 45° output, use the fixed-orientation version (Hershberger–Snoeyink; Widmayer, Wu, Wong, "On some distance problems in fixed orientations", SIAM J. Comput. 1987).
- **Pain points:**
  - 6 (any-angle and arcs).
  - 2, because it gives exact placement in the channel instead of lattice snapping.
  - Better wirelength and fewer bends.
- **Why this differs from your weak position-based-dynamics (PBD) rubber band.**
  - PBD is an iterative relaxation that has to fight clearance constraints.
  - The funnel method is an exact linear-time computation of the global optimum per homotopy class, and satisfies constraints by construction.
- **Main risk.**
  - Pull-tight can create new conflicts with wires in other homotopy classes nearby, so fixed-point iteration may be needed.
  - Multi-terminal nets need Steiner points released as free variables (Fermat points in the any-angle case).
  - Arc-to-arc clearance checks need a robust kernel (idea 20).
- **First experiment.** Post-process the current lattice output: extract each net's sleeve and run the funnel with discs inflated per net class. Measure wirelength drop, bend count and checker violations. This is 2–3 days of work with no change to the router.
- **Rating:** V: H, E: M.

### 5. Via-minimizing layer assignment via planar max-cut (route in 2D first, then assign layers)
- **References:**
  - Chen, Kajitani, Chan, "A graph-theoretic via minimization algorithm for two-layer printed circuit boards", IEEE TCAS 30(5) 1983.
  - Pinter, "Optimal layer assignment for interconnect", 1983 (?, venue).
  - Hadlock, "Finding a maximum cut of a planar graph in polynomial time", SIAM J. Comput. 1975.
  - Grötschel et al., "Via minimization with pin preassignments and layer preference", ZAMM 1989.
- **How it maps.**
  - Compute a topological 2D routing with crossings allowed (idea 2, on a single virtual layer).
  - Build the conflict graph: one node per wire piece; crossing pieces must lie on different layers.
  - For 2 layers this is bipartization. Every odd cycle needs a via, and minimizing vias is a max-cut problem on a planar graph, which is polynomial (Hadlock / T-joins).
  - Add weights for layer preference and for pour fragmentation (idea 6): prefer short hops on the layer whose pour would otherwise be cut.
  - For 4 layers, use k-cut heuristics with the pair of layers next to each plane preferred.
- **Pain points:** 1 (layer choice is where pour damage is decided), 5.
- **Why beat the lattice.** Negotiated A* over (layer, node) decides layers greedily per net. Max-cut decides them globally with an optimality guarantee for the via count.
- **Main risk.** The 2D topology has to anticipate via space. Vias inserted after the fact need room, so reserve via sites in the triangles.
- **First experiment.** Take the lattice's 2-layer result, project it to 2D, re-solve the layer assignment by max-cut (exact via planar T-join, or just an ILP at this size), and compare via count and pour island count.
- **Rating:** V: M–H, E: M.

---

## B. Pour connectivity (pain point 1, the largest)

### 6. Pour connectivity as planar duality: Betti-number accounting, a face graph, and a minimum spanning tree to choose which net yields
- **References:**
  - Alexander duality and the Euler formula (standard).
  - Edelsbrunner & Harer, *Computational Topology*, AMS 2010.
  - KiCad zone-fill semantics: clearance, minimum thickness, island removal.
- **How it maps.**
  - Fix one layer L and the pour net G. Let O be the union of all non-G copper on L (tracks, pads, vias) plus the board edge and keepouts, each inflated by `c_z + t_min/2`. Here c_z is the zone clearance and t_min the zone minimum thickness.
  - The pour on L is then, up to smoothing, the complement of O inside the board. Its connected pieces correspond one-to-one to the holes of O plus one, i.e. b₀(pour) = b₁(O) + 1, with the board frame counted as part of O.
  - The key local rule: two objects are pour-adjacent if their gap is less than 2c_z + t_min.
    - Keep a union-find over O's components per layer.
    - Inserting a track that touches the same O-component at two distinct contacts closes a loop. That is b₁ += 1, i.e. one more pour face.
    - Generally, Δb₁ = (#contacts) − (#distinct components touched).
    - This is exact, O(α(n)) per contact, and needs no polygon clipping.
  - **Cross-layer (2-layer, both poured).**
    - Build the face graph F: nodes are pour faces on top and bottom.
    - Edges come from existing stitch vias and through-hole G pads.
    - Potential edges are stitch sites, weighted 0 if a via fits (idea 7) and ∞ otherwise.
    - The pour is fine if and only if F restricted to real and potential edges is connected, and every node contains or reaches a G pad.
  - **Which net yields.**
    - Add dual edges: for adjacent faces separated only by track t of net n, add an edge weighted by n's rip-up or reroute cost.
    - The minimum spanning tree over F (stitch edges cost 0) names the cheapest set of signal pieces whose removal restores connectivity.
    - Equivalently, it is a shortest path in the dual. This is precisely the dual-cut view.
- **Router pricing.**
  - A* can't carry contact state cheaply. Instead, after each net is routed, compute Δb₁ and the faces with no possible stitch.
  - Add PathFinder history cost on the contact zones that closed loops. Or pre-mark "pour spine" edges (idea 7) and price crossings of them.
- **Why beat the current approach.** Today islands are stitched afterwards. This measures the damage while routing, exactly and cheaply, and gives a principled answer to which net yields.
- **Main risk.**
  - Round-offset versus polygon approximation near the threshold, and thermal-relief spokes. Validate against KiCad's real fill.
  - Contact via inflated-union adjacency is conservative near sharp corners.
- **First experiment.** On the stalled 2-layer boards, compute b₁(O) per layer, the face graph and its MST. Check that the predicted island count matches KiCad's fill. Then feed Δb₁ into the history cost and measure island count and completion.
- **Rating:** V: H, E: S–M.

### 7. Medial axis (Voronoi of segments) as a "room graph": pour spine, stitch sites, and channel widths
- **References:**
  - Voronoi diagrams of line segments: Held's VRONI; Boost.Polygon's Voronoi; CGAL's segment Delaunay graph.
  - Chazal & Lieutier, "The λ-medial axis", Graphical Models 2005: stable pruning of the medial axis.
  - Local feature size (Amenta & Bern).
- **How it maps.**
  - Compute the medial axis of free space per layer. Each medial-axis point carries its inscribed radius ρ.
  - **(a) Stitch site test.** A point x can host a G via connecting a top face to the bottom main face if and only if ρ_top(x) ≥ r_via + c_z and ρ_bot(x) ≥ r_via + c_z. The via is G copper, so on the bottom it just has to lie in the main face; the ρ_bot condition is a safe side of that.
  - **(b) Pour spine.** Take the maximum-bottleneck spanning tree on the medial axis connecting all G pads, per layer, joined across layers at stitch sites. Reserve it: a signal may cross a top spine edge only if the bottom spine covers that area (a stitchable crossing).
    - This is a geometric, principled version of your "plane skeleton", and it picks the widest necks.
  - **(c) Channel capacity.** Capacity is ⌊(2ρ − cl) / (w + cl)⌋ along medial-axis edges. It is a coarse corridor graph where the bottleneck width tells which net classes can use a channel.
- **Pain points:** 1, 2, 4. Placement can maximize the minimum ρ on the needed corridors.
- **Complement to the lattice.** Use it for prices and reservations; the lattice still routes.
- **Main risk.** The medial axis is unstable (spurious branches), so λ-pruning is needed. Segment-Voronoi robustness also matters, so use integer coordinates in nm.
- **First experiment.** Compute stitchable islands versus unstitchable ones on current outputs. If most failures are unstitchable islands, the spine reservation is the fix.
- **Rating:** V: H, E: M.

### 8. Persistent homology over clearance and inflation radius (diagnostics and per-net-class corridors)
- **References:**
  - Edelsbrunner, Letscher, Zomorodian, "Topological persistence and simplification", DCG 2002.
  - Alpha shapes and alpha complexes (Edelsbrunner & Mücke 1994), which can be computed on the CDT.
- **How it maps.**
  - Use the filtration of O_r (obstacles inflated by r) per layer.
  - H₁ births and deaths of O_r are exactly the radii at which channels close, which by Alexander duality is H₀ of free space.
  - One barcode gives: (i) every pour neck with its death radius, and whether it dies before `c_z + t_min/2`; (ii) which net classes fit through each channel, i.e. corridor graphs for all classes in one computation; (iii) sensitivity analysis ("if clearance were 0.15 mm instead of 0.2 mm, how many islands would merge").
- **Pain points:** 1, 7, and debugging.
- **Main risk.** It mostly repackages the medial axis (idea 7) in a stable form. Its value is diagnosis and choosing what to relax, not routing itself.
- **First experiment.** Compute an alpha-complex barcode on the pad and courtyard CDT. Plot the channel death-radius histogram against the net-class half-widths to see immediately where the lattice can't possibly help.
- **Rating:** V: M, E: S.

### 9. Morphological pour prediction (offsets and straight skeleton)
- **References:**
  - Aichholzer, Aurenhammer, Alberts, Gärtner, "A novel type of skeleton for polygons", J.UCS 1995.
  - Offset libraries: Clipper, and the Rust `i_overlay` crate.
- **How it maps.** KiCad's fill is roughly an opening by t_min/2 of (zone − obstacles⊕c_z), followed by island removal. Predict it exactly during routing, incrementally per tile. The straight skeleton (mitred offsets) or the medial axis (round offsets) gives each piece's vanishing radius in one pass.
- **Pain point:** 1. It closes the gap between "topologically connected" and "KiCad removes the sliver".
- **Main risk.** Low novelty; it is really a prerequisite for idea 6.
- **Rating:** V: M, E: S.

---

## C. Fine-pitch escape and detail resolution (pain point 2)

### 10. BGA escape by flow on the pin-grid triangulation with exact diagonal capacities, then channel-exact geometry, plus pin swap
- **References:**
  - Yu & Dai, "Single-layer fanout routing and routability analysis for ball grid arrays", ICCAD 1995.
  - Yu, Darnauer, Dai, ICCAD 1996 (as in idea 1).
  - Yan & Wong, "Correctly modeling the diagonal capacity in escape routing", IEEE TCAD 31(2) 2012.
  - Ozdal & Wong, "Algorithms for simultaneous escape routing and layer assignment of dense PCBs", IEEE TCAD 25(8) 2006.
  - Kong, Yan, Wong, "Optimal simultaneous pin assignment and escape routing for dense PCBs", ASP-DAC 2010.
  - "Ordered escape routing with consideration of differential pair and blockage", ACM TODAES 2018.
- **How it maps.**
  - The escape region is the BGA pad array plus the dogbone via sites.
  - Run a min-cost flow network: nodes are pads, channel crossings and via sites; capacities come from the exact channel widths. Include Yan–Wong's correction for diagonal capacity shared between orthogonal and diagonal channels.
  - The flow gives the topology and the layer (Ozdal–Wong also minimizes crossings outside the BGA).
  - Geometry is then solved per channel, not per lattice node. In a straight channel of width d with k tracks, positions follow from the clearance sums, so a 0.01 mm slack is used exactly.
  - Pin swap (swappable FPGA or connector pins) is handled by the flow's free choice of sink.
- **Why beat the lattice.** The lattice has to align its pitch and phase with the channels. Here the channel is the unit, so the runtime doesn't depend on the slack.
- **Main risk.**
  - Via-in-pad or dogbone patterns vary by footprint.
  - Mixed pad shapes (rounded rectangles) change channel widths along the channel, so compute widths from real polygons.
- **First experiment.** A single 0.8 mm or 0.75 mm BGA test: flow escape to the array boundary, write exact tracks, and have the exact checker verify them. Compare against the lattice at 0.075 mm for completion and runtime.
- **Rating:** V: H, E: M.

### 11. A non-uniform grid built from the geometry
- **References:**
  - Zheng, Lim, Iyengar, "Finding obstacle-avoiding shortest paths using implicit connection graphs", IEEE TCAD 15(1) 1996.
  - Hightower, "A solution to line-routing problems on the continuous plane", DAC 1969.
  - Mikami & Tabuchi, 1968.
  - "A multi-layer gridless area routing algorithm based on non-uniform-grid graph" (?, authors and venue unverified).
- **How it maps.**
  - Replace uniform pitch with escape lines: for each obstacle edge and each net class c, add the line offset by `cl_c + w_c/2`. Add 45° lines through inflated corners.
  - Nodes are intersections, generated implicitly and lazily during A*.
  - Every legal "hug" position is then exactly a node, so the 0.01 mm slack is representable without a 0.075 mm lattice everywhere.
  - A variant is a lattice whose pitch adapts to the local feature size (from ideas 7 and 8): fine only where the inscribed radius ρ is small.
- **Pain points:** 2, 5.
- **Main risk.** Per-class lines multiply the graph (classes × obstacles), and the number of 45° intersections can explode. Mitigate by restricting to lines near obstacles.
- **First experiment.** Swap the lattice for an implicit Hanan-plus-45° graph around one BGA only. Count expanded nodes versus the uniform lattice.
- **Rating:** V: M–H, E: M.

### 12. Coarse lattice for topology, then exact legalization by LP (compaction)
- **References:**
  - Maley, *Single-Layer Wire Routing and Compaction*, MIT Press 1996.
  - "A generic algorithm for one-dimensional homotopic compaction", Algorithmica (?, authors unverified).
  - Constraint-graph compaction (classic VLSI).
- **How it maps.**
  - Route on a 0.15–0.2 mm lattice with capacity-aware channels, where a channel means "2 tracks here" rather than exact positions.
  - Then fix the homotopy and solve an LP or QP for vertex positions: a clearance constraint for each (segment, obstacle-vertex) pair seen across free space, linearized along the separating direction; the objective is wirelength or spreading.
  - This keeps the topology and makes the geometry exact.
- **Pain points:** 2, 5 (a coarser lattice runs roughly quadratically faster).
- **Main risk.** Linearizing clearance for oblique segments is only locally valid, so iterate. It is infeasible if the topology is overfull, but idea 3 detects that first.
- **First experiment.** A BGA quadrant: route at 2× the current pitch while allowing capacity-2 channels, then legalize with an LP (`good_lp` plus HiGHS in Rust).
- **Rating:** V: M–H, E: M.

---

## D. Runtime and search (pain point 5)

### 13. Corner-stitched tiles or "expansion rooms" as macro-nodes
- **References:**
  - Ousterhout, "Corner stitching: a data-structuring technique for VLSI layout tools", IEEE TCAD 1984.
  - Margarino et al., "A tile-expansion router", IEEE TCAD 1987.
  - Ohtsuki, "Gridless routers — new wire routing algorithms based on computational geometry", 1985 (?, venue).
  - Freerouting (Alfons Wirtz), which does maze search over expansion rooms.
  - "Shortest path search using tiles and piecewise linear cost propagation", IEEE TCAD (?, authors unverified).
- **How it maps.** Most of a 4-layer board is empty. Maximal empty tiles per layer, per net class (from inflated obstacles), become nodes. A* inside a tile uses piecewise-linear cost propagation along tile edges, so costs are exact within a tile. Congestion is priced per tile.
- **Pain point:** 5. There are orders of magnitude fewer nodes in open regions. Keep the lattice only in "narrow" tiles.
- **Main risk.** Per-class inflation means one tile plane per class. Tile fragmentation after many routes; mitigate with tile merging. Piecewise-linear cost propagation is fiddly.
- **Rating:** V: M, E: M–L.

### 14. Obstacle-aware any-angle distance as the A* heuristic (Polyanya on a navigation mesh)
- **References:**
  - Cui, Harabor, Grastien, "Compromise-free pathfinding on a navigation mesh", IJCAI 2017 (Polyanya).
  - Harabor, Grastien, Öz, Aksakalli, "Optimal any-angle pathfinding in practice", JAIR 2016 (Anya).
  - Demyen & Buro, "Efficient triangulation-based pathfinding", AAAI 2006.
- **How it maps.**
  - Congested A* cannot use Polyanya directly, because weighted regions are hard.
  - But the obstacle-aware Euclidean shortest distance on the CDT of class-inflated obstacles is an admissible heuristic for any cost of the form ≥ length. It is much tighter than octile distance around big connectors, pour cutouts and board slots.
  - Precompute a distance field from each net's target on the CDT (one Dijkstra over triangles or funnel windows), then look it up in lattice A*.
- **Pain point:** 5 (fewer expansions), essentially for free.
- **Main risk.** The gain is small where congestion prices dominate the true cost, so combine it with landmark (ALT) heuristics.
- **First experiment.** Log the A* expansions per net with and without the CDT heuristic.
- **Rating:** V: M, E: S.

### 15. Local push-and-shove for the last 10 %
- **References:** the KiCad PNS router (Tomasz Włostowski, CERN): walkaround and shove with octagonal clearance hulls.
- **How it maps.** When PathFinder stalls, try gridless local moves instead of global rip-up: shove neighbouring tracks sideways within their homotopy class to open a passage, and verify with the exact checker. This complements the lattice rather than replacing it.
- **Pain points:** 1 (the stall), 5.
- **Main risk.** Cascading shoves. Licensing is also a question: KiCad is GPL, so porting ideas is fine but copying code depends on your licence.
- **Rating:** V: M, E: M.

---

## E. Placement (pain points 3 and 4)

### 16. Legality with real courtyard polygons: no-fit polygons, penetration depth, phi-functions
- **References:**
  - Bennell & Oliveira, "The geometry of nesting problems: a tutorial", EJOR 2008.
  - Egeblad, Nielsen, Odgaard, "Fast neighborhood search for two- and three-dimensional nesting problems", EJOR 183 (2007). Exact 1D translation moves that minimize overlap.
  - Imamichi, Yagiura, Nagamochi, "An iterated local search algorithm based on nonlinear programming for the irregular strip packing problem", Discrete Optimization 6 (2009). Separation by penetration depth.
  - Phi-functions: Chernov, Stoyan, Romanova, CGTA 2010 (?, exact title).
  - **Rust:** `jagua-rs`, a collision detection engine for 2D nesting (Gardeyn & Wauters, INFORMS J. Computing). Also "An open-source heuristic to reboot 2D nesting research", arXiv:2509.13329 (the "sparrow" solver, Rust).
- **How it maps.**
  - Each footprint has its real courtyard polygon, plus a routing halo derived from pin demand rather than a uniform one: offset the courtyard side by (#escaping pins on that side) × (w + cl) / (#layers).
  - Two-sided boards: surface-mount parts collide only on the same side, through-hole parts on both. Each side is its own nesting sheet, and through-hole parts appear on both sheets.
  - Legal reference positions for part Q are the inner-fit polygon of the board minus the union of NFP(P_i, Q_θ). The nearest legal point to the analytic target is an exact query.
  - For legalization, replace simulated annealing with guided local search on penetration depth (Egeblad / Imamichi).
  - For free rotation, sample 8–24 angles for the NFPs, or use phi-functions for a continuous θ.
- **Pain point:** 3, directly. Rectangles waste the L- and T-shaped slack of real courtyards.
- **Main risk.** Non-convex Minkowski sums are costly. `jagua-rs` avoids NFPs altogether with quadtree surrogate collision checks.
- **First experiment.** Feed the dense two-sided failure cases (courtyards, board outline, the ePlace positions as targets) into `jagua-rs` / sparrow-style overlap minimization and count legal outcomes.
- **Rating:** V: H, E: S–M.

### 17. Routability-aware placement via geometric cut capacities, plus a crossing proxy for 2 layers
- **References:**
  - RUDY: Spindler & Johannes, DATE 2007. This is the density baseline.
  - Yu–Dai cut analysis (as in idea 10).
  - Leiserson–Maley cuts (as in idea 3).
- **How it maps.**
  - Build a CDT on courtyards (placement-level obstacles). For each edge between two parts: gap g, capacity c = g · layers / (w + cl), and demand = the expected number of net Steiner edges crossing it (a smoothed crossing indicator, differentiable in positions).
  - Penalize Σ max(0, demand − c)² inside the electrostatic objective. The gap g is differentiable, and CDT re-triangulation is needed only when the topology changes.
  - On 2-layer boards, add the frustration of the Steiner-edge crossing graph: odd cycles force vias, and long crossings cut pours.
- **Pain point:** 4. Wirelength-optimal placement is not routable because RUDY ignores where the bottlenecks between parts are; cuts measure exactly that.
- **Main risk.** Noisy gradients when the CDT flips. Use a fixed-topology gradient between re-triangulations.
- **First experiment.** Offline, correlate the maximum CDT cut overflow of final placements with router completion over your benchmark set, compared with RUDY's peak. If the correlation is better, integrate it.
- **Rating:** V: H, E: M.

---

## F. Net topology and high-speed features (pain point 6)

### 18. Octilinear, obstacle-avoiding Steiner trees for multi-terminal nets
- **References:**
  - GeoSteiner (Warme, Winter, Zachariasen).
  - FLUTE (Chu & Wong, IEEE TCAD 2008).
  - Kahng, Măndoiu, Zelikovsky, "Highly scalable algorithms for rectilinear and octilinear Steiner trees", ASP-DAC 2003.
  - Lin et al., "Efficient obstacle-avoiding rectilinear Steiner tree construction", ISPD 2007 (OARSMT).
- **How it maps.**
  - Before PathFinder, fix a Steiner topology per net on the CDT: obstacle-avoiding, pour-aware (Steiner points avoid spine edges from idea 7).
  - For pour nets, the octilinear Steiner tree is the ideal plane skeleton.
  - In the geometric phase (idea 4), Steiner points move freely.
- **Pain points:** 1, 4, 5.
- **Rating:** V: M, E: S–M.

### 19. Differential pairs and length matching, done geometrically
- **References:**
  - Yan & Wong, "BSG-Route: a length-matching router for general topology", ICCAD 2008 (gridless).
  - Ozdal & Wong, "A length-matching routing algorithm for high-performance PCBs", IEEE TCAD 2006 (?, year).
  - Fang et al., "Obstacle-aware length-matching routing for any-direction traces in printed circuit board", DAC 2024 / arXiv:2407.19195.
  - The `cavalier_contours` Rust crate: exact offsets of polylines with arcs.
- **How it maps.**
  - **Diff pairs.** In the topology layer, treat the pair as one wire of width 2w + s, sharing one homotopy class. Realize the members as the offset curves ±(s + w)/2 of the centreline.
    - An exact identity for arc corners: intra-pair skew = (s + w) · Σ signed turning angles. For a 45° mitred corner the per-corner term is 2·tan(θ/2) instead of θ.
    - So skew is a topological and geometric invariant of the centreline (its net turning). Choose sleeves with near-zero net turning, or add one compensation bump per unit of turning.
  - **Length matching.** Meanders go into the free space along the trace. The available amplitude is bounded by the local inscribed radius ρ from the medial axis (idea 7), which gives the maximum insertable length per segment in closed form before any search.
  - **Buses.** River routing (single layer, order preserved) is solvable optimally in linear time: Tompa, "An optimal solution to a wire-routing problem", 1980; Dolev, Karplus, Siegel, Strong, Ullman, "Optimal wiring between rectangles", STOC 1981. Detect connector-to-connector buses and route them as one fat bundle.
- **Pain point:** 6.
- **Main risk.** Breakout at pads (neck-down, uncoupled stubs) is special-case geometry.
- **First experiment.** Route diff pairs as fat wires through the existing corridor router, then offset and check with the exact checker.
- **Rating:** V: M–H (feature gap), E: M.

---

## G. Foundations

### 20. Exact integer kernel and robust predicates
- **References:**
  - Shewchuk, "Adaptive precision floating-point arithmetic and fast robust geometric predicates", DCG 1997.
  - Rust crates: `robust`, `spade` (CDT, exact predicates), `i_overlay` (integer Boolean operations and offsets), `cavalier_contours`.
- **How it maps.**
  - Keep all geometry in KiCad-native integer nanometres. Orientation tests are exact in i128; in-circle tests need filtered f64 with an exact fallback.
  - Arcs have irrational tangency points. Represent them as (centre, radius, endpoints) snapped to nm, and inflate the clearance by a sagitta or snap error of about 2 nm, which is conservative.
  - This is a prerequisite for ideas 1, 4, 6 and 7. Degenerate CDTs from aligned BGA pads are the norm, not the exception.
- **Rating:** V: M (enabling), E: S.

### 21. Configuration-space view for placement plus routing: pin-access polygons
- A short idea. For each pad, compute its access region per layer: the set of legal track-end positions (the pad inflated inward, minus neighbours inflated by class clearance). Feed it as terminals to the CDT router and the escape flow instead of pad centres.
- This removes the need for lattice phase alignment (currently pad centres have to fall on lattice nodes) and makes off-centre entry possible, which 0.01 mm slack often needs.
- **Pain point:** 2.
- **Rating:** V: M, E: S.

---

## Top 5, with justification

1. **Pour connectivity by duality (idea 6) plus the medial-axis spine and stitch sites (idea 7).**
   - Pain point 1 is the biggest completion blocker.
   - The Betti-number bookkeeping (Δb₁ from union-find contacts with threshold 2c_z + t_min) is exact, cheap and incremental.
   - The face-graph MST directly answers "which net yields" for pours.
   - The stitch-site test turns "stitch islands afterwards" into "only create islands that can be stitched".
   - Effort is small to medium, it fits the existing PathFinder as history costs, and it is testable on existing outputs in days.
2. **CDT cross-section capacity graph as the global and topological router (idea 1), with cut certificates (idea 3).**
   - It extends your most promising prototype.
   - It turns capacities from lattice counts into exact distances, and replaces tile corridors with homotopy sleeves.
   - Run on the same structure, the cut and LP-dual tests give sound infeasibility proofs and Lagrangian choices of which net yields (pain point 7).
   - It also speeds up 4-layer runs: global rip-up happens on about 10⁴ triangles, and the lattice runs only inside sleeves.
3. **BGA escape by exact-capacity flow plus channel-exact geometry (idea 10), with LP legalization as the generalization (idea 12).**
   - This removes the lattice-resolution trade-off exactly where it hurts: the slack becomes arithmetic, not pitch.
   - The literature (Yu–Dai, Yu–Darnauer–Dai, Yan–Wong, Ozdal–Wong, Kong–Yan–Wong) is mature and precisely on target.
   - Pin swap comes free from the flow formulation.
4. **Real-courtyard nesting legality (idea 16), plus cut-capacity routability in the placer (idea 17).**
   - Pain point 3 is a hard failure (no legal placement), and off-the-shelf Rust machinery exists (`jagua-rs`, sparrow) that handles irregular polygons and rotations.
   - A cheap follow-on attacks pain point 4 by replacing RUDY-style density with the same CDT cut measure the router uses. Placer and router then optimize one notion of "room".
5. **Shortest homotopic fat-path realization with arcs (idea 4).**
   - It is the geometry half of the topology-first design. Without it, topological routing still depends on the lattice for shapes.
   - It gives any-angle and arc output that is DRC-correct by construction (tangent-and-arc paths around discs of radius cl + w/2).
   - It fixes what PBD could not, because the result is an exact per-homotopy optimum rather than a relaxation.
   - It is the substrate for diff pairs (offset curves, skew = (s + w)·Σ turning) and length matching (idea 19).
   - It can be tried first as a pure post-processor on the current lattice output.

**Suggested order:** 6/7 first (quick win on pain point 1), then 20 (the integer kernel, which the rest needs), 4 as a post-processor, and 16 in parallel. After that come 1/3, which is the structural change, and 10 on top of it.

---

## Sources checked
- [Yan & Wong, diagonal capacity (TCAD 2012)](https://dl.acm.org/doi/10.1145/3185783)
- [Yu & Dai, BGA fanout, ICCAD 1995](https://ieeexplore.ieee.org/document/480175/)
- [Yu, Darnauer, Dai, interchangeable pin routing, ICCAD 1996](https://ieeexplore.ieee.org/document/571349/)
- [Kallmann, LCT, ACM TOG 2014](https://dl.acm.org/doi/10.1145/2580947)
- [Duncan et al., Drawing with fat edges](https://www.worldscientific.com/doi/abs/10.1142/S0129054106004315)
- [Maley, homotopic routability (Algorithmica 1996)](https://link.springer.com/article/10.1007/BF01942604)
- [Schrijver, disjoint homotopic paths (DCG)](https://link.springer.com/article/10.1007/BF02574704)
- [Egeblad et al., EJOR 2007](https://www.sciencedirect.com/science/article/abs/pii/S037722170600302X)
- [Imamichi et al., Discrete Optimization 2009](https://www.sciencedirect.com/science/article/pii/S1572528609000218)
- [jagua-rs](https://github.com/JeroenGar/jagua-rs)
- [arXiv:2509.13329, sparrow](https://arxiv.org/pdf/2509.13329)
- [Polyanya, IJCAI 2017](https://www.ijcai.org/proceedings/2017/0070.pdf)
- [Chen, Kajitani, Chan, via minimization](https://www.semanticscholar.org/paper/A-graph-theoretic-via-minimization-algorithm-for-Chen-Kajitani/317e080bec3b8cbd701882a5a9f7faee5d5d1f64)
- [Zheng, Lim, Iyengar, TCAD 1996](https://ieeexplore.ieee.org/document/486276/)
- [Ozdal & Wong, TCAD 2006](https://dl.acm.org/doi/10.1109/TCAD.2005.857376)
- [Kong, Yan, Wong, ASP-DAC 2010 (via search listing)](https://dl.acm.org/doi/10.5555/1326073.1326154)
- [SURF topological routing, DAC 1991](https://dl.acm.org/doi/pdf/10.1145/127601.127622)
- [Dayan thesis](https://www.semanticscholar.org/paper/RUBBER-BAND-BASED-TOPOLOGICAL-ROUTER-Dayan-Cruz/91ce7726d0b103db47ab5db433ed75b538e6e7f8)
- [Seong et al., arXiv:2105.07892](https://arxiv.org/abs/2105.07892)
- [Fang et al., arXiv:2407.19195](https://arxiv.org/abs/2407.19195)
- [Hershberger & Snoeyink, CGTA 1994](https://www.sciencedirect.com/science/article/pii/0925772194900108)
- [Efrat, Kobourov, Lubiw, CGTA 2006](https://dblp.org/rec/journals/comgeo/EfratKL06.html)
- [Gao et al., continuous homotopic one-layer routing](https://link.springer.com/chapter/10.1007/3-540-50335-8_24)
- [BSG-Route, ICCAD 2008](https://ieeexplore.ieee.org/document/4681621/)
- [Kahng, Măndoiu, Zelikovsky, ASP-DAC 2003](https://ieeexplore.ieee.org/document/1195132/)
- [TopoR](https://en.wikipedia.org/wiki/TopoR)

Cited from memory with high confidence, not re-checked: Okamura–Seymour 1981, Hadlock 1975, Tompa 1980, Dolev et al. 1981, Widmayer–Wu–Wong 1987, Ousterhout 1984, Shewchuk 1997, Edelsbrunner–Letscher–Zomorodian 2002, Chazal–Lieutier 2005, Aichholzer et al. 1995, FLUTE, GeoSteiner, Lin et al. ISPD 2007, RUDY, Anya (JAIR 2016), Demyen–Buro 2006, Hightower 1969. Venue or details marked (?) are uncertain: Pinter 1983, Ohtsuki 1985, Margarino 1987 details, the Iri / Onaga–Kakusho attribution, the phi-function paper title, and the Ozdal–Wong length-matching year.
