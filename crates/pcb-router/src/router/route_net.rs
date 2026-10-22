// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Routing one net: its tree, one multi-target search at a time.

use super::*;

/// `route_net`'s absorb: takes a group of terminals and retained branches
/// (by its root) into a tree.
type Absorb<'a> = dyn Fn(&mut Scratch, &mut NetState, &mut Vec<Node>, &mut Vec<bool>, usize) + 'a;

impl Router {
    /// Debug: with `PCB_ROUTER_DUMP=x0,y0,x1,y1` (mm), prints the lattice
    /// of that region per layer: `#` static block, `x` the nets' own
    /// pads, `.` free, a digit the occupancy, `*` an occupied node of one
    /// of `nets`, `v` where a via may not go.
    pub(super) fn dump_region(&self, nets: &[NetId]) {
        let Some(spec) = std::env::var_os("PCB_ROUTER_DUMP") else {
            return;
        };
        let bounds: Vec<f64> = spec
            .to_string_lossy()
            .split(',')
            .filter_map(|v| v.parse().ok())
            .collect();
        if bounds.len() != 4 {
            return;
        }
        let Some((x0, y0, x1, y1)) = self.grid.node_range(crate::geometry::Aabb {
            minimum: [bounds[0], bounds[1]],
            maximum: [bounds[2], bounds[3]],
        }) else {
            return;
        };
        let layers = self.board.layer_count;
        let mut own_nodes: HashSet<(usize, u32)> = HashSet::new();
        for net in nets {
            for branch in &self.nets[*net as usize].branches {
                for node in &branch.nodes {
                    own_nodes.insert((node.layer as usize, node.cell));
                }
            }
        }
        for layer in 0..layers {
            eprintln!(
                "  layer {layer}, x {:.2}..{:.2}, y {:.2}..{:.2}",
                bounds[0], bounds[2], bounds[1], bounds[3]
            );
            for y in y0..=y1 {
                let mut line = String::new();
                for x in x0..=x1 {
                    let cell = self.grid.index(x, y);
                    let class = self.board.nets[nets[0] as usize].class;
                    let allowed = self.statics[class].trace_at(layer, cell);
                    let occupancy = self.occupancy[self.map_index(class, layer)][cell];
                    line.push(if own_nodes.contains(&(layer, cell as u32)) {
                        '*'
                    } else if allowed == crate::grid::BLOCKED {
                        '#'
                    } else if allowed != crate::grid::FREE {
                        if nets.iter().any(|net| allowed == crate::grid::owner(*net)) {
                            'x'
                        } else {
                            'o'
                        }
                    } else if occupancy > 0 {
                        char::from_digit(occupancy.min(9) as u32, 10).unwrap()
                    } else if self.statics[class].via_blocked.get(cell) {
                        'v'
                    } else {
                        '.'
                    });
                }
                eprintln!("  {line}");
            }
        }
    }

    pub(super) fn window(&self, net: NetId, growth: f64) -> (usize, usize, usize, usize) {
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        for terminal in &self.board.nets[net as usize].terminals {
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(terminal.anchor[axis]);
                maximum[axis] = maximum[axis].max(terminal.anchor[axis]);
            }
        }
        let extent = (maximum[0] - minimum[0]).max(maximum[1] - minimum[1]);
        let margin = self.config.window_margin.max(0.35 * extent) * growth;
        let bounds = crate::geometry::Aabb { minimum, maximum }.inflated(margin);
        self.grid
            .node_range(bounds)
            .unwrap_or((0, 0, self.grid.nx - 1, self.grid.ny - 1))
    }

    /// Connects all terminals of `net` into one tree, reusing any branches
    /// the net still has. With `hard`, nodes claimed by other nets are
    /// impassable and a partial result may be kept.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn route_net(
        &self,
        scratch: &mut Scratch,
        net: NetId,
        net_state: &mut NetState,
        present: f32,
        hard: bool,
        window_growth: f64,
    ) {
        scratch.ensure(
            self.grid.cells() * self.board.layer_count,
            self.grid.cells(),
        );
        let terminal_count = self.board.nets[net as usize].terminals.len();
        let retained = net_state.branches.clone();
        let keep = vec![true; retained.len()];
        let hosts = junction_hosts(&retained, &keep);

        // Elements are terminals, retained branches, and finally the pours.
        let has_plane = !net_state.plane.is_empty();
        let plane_element = terminal_count + retained.len();
        let mut parent: Vec<usize> =
            (0..terminal_count + retained.len() + has_plane as usize).collect();
        if has_plane {
            for (slot, on_plane) in parent
                .iter_mut()
                .zip(&net_state.on_plane)
                .take(terminal_count)
            {
                if *on_plane {
                    *slot = plane_element;
                }
            }
        }
        for (index, branch) in retained.iter().enumerate() {
            let element = terminal_count + index;
            let ends = [
                (branch.start_terminal, branch.nodes[0]),
                (branch.end_terminal, *branch.nodes.last().unwrap()),
            ];
            for (terminal, node) in ends {
                let other =
                    if terminal == PLANE_TERMINAL {
                        // While pads must reach the main piece, a stub joins the
                        // plane only where it still ends in that piece: the
                        // signals may have cut its end off since (k30's +5V: 22
                        // pads stranded on stubs into islands or no fill at all,
                        // which every reroute kept as connected).
                        if !net_state.plane_target.is_empty()
                            && !net_state.plane_target.get(node.layer as usize).is_some_and(
                                |mask| mask.get(node.cell as usize).copied().unwrap_or(false),
                            )
                        {
                            continue;
                        }
                        plane_element
                    } else if terminal == FREE_END {
                        continue;
                    } else if terminal != NO_TERMINAL {
                        terminal as usize
                    } else if let Some(host) = hosts.get(&node) {
                        terminal_count + host
                    } else {
                        // A junction whose host is gone: the end is free.
                        continue;
                    };
                let (a, b) = (find(&mut parent, element), find(&mut parent, other));
                parent[a] = b;
            }
        }
        let component: Vec<usize> = (0..parent.len())
            .map(|element| find(&mut parent, element))
            .collect();

        scratch.tree.clear();
        let mut tree: Vec<Node> = Vec::new();
        let mut in_tree = vec![false; parent.len()];
        let cells = self.grid.cells();
        let state_of = |node: Node| node.layer as usize * cells + node.cell as usize;
        let absorb = |scratch: &mut Scratch,
                      net_state: &mut NetState,
                      tree: &mut Vec<Node>,
                      in_tree: &mut Vec<bool>,
                      root: usize| {
            for element in 0..component.len() {
                if component[element] != root || in_tree[element] {
                    continue;
                }
                in_tree[element] = true;
                if has_plane && element == plane_element {
                    continue;
                }
                if element < terminal_count {
                    for node in net_state.terminal_nodes[element].clone() {
                        scratch.tree.insert(state_of(node) as u32, element as u16);
                        tree.push(node);
                    }
                    net_state.connected[element] = true;
                } else {
                    for node in &retained[element - terminal_count].nodes {
                        if let std::collections::hash_map::Entry::Vacant(entry) =
                            scratch.tree.entry(state_of(*node) as u32)
                        {
                            entry.insert(NO_TERMINAL);
                            tree.push(*node);
                        }
                    }
                }
            }
        };
        if has_plane {
            self.connect_to_plane(
                scratch,
                net,
                net_state,
                &retained,
                &component,
                plane_element,
                present,
                hard,
            );
            return;
        }
        let first_live = (0..terminal_count)
            .find(|terminal| !net_state.terminal_nodes[*terminal].is_empty())
            .unwrap_or(0);
        absorb(
            scratch,
            net_state,
            &mut tree,
            &mut in_tree,
            component[first_live],
        );

        // Groups holding a terminal no path reaches (`unreachable`) are
        // left out of the negotiation's searches; the hard searches try
        // everything (a multi-target search fails only when no target is
        // left that it reaches).
        let mut left_out = vec![false; component.len()];
        if !hard {
            for (terminal, unreachable) in net_state.unreachable.iter().enumerate() {
                if *unreachable
                    && terminal < terminal_count
                    && component[terminal] != component[first_live]
                {
                    left_out[component[terminal]] = true;
                }
            }
        }
        let pitch = self.grid.pitch as f32;
        loop {
            let mut targets: Vec<(Node, u16)> = Vec::new();
            let mut points: Vec<(i32, i32, f32)> = Vec::new();
            let mut exact_points = true;
            for element in 0..component.len() {
                if in_tree[element] || left_out[component[element]] {
                    continue;
                }
                if element < terminal_count {
                    for node in &net_state.terminal_nodes[element] {
                        targets.push((*node, element as u16));
                    }
                    let anchor = self.board.nets[net as usize].terminals[element].anchor;
                    points.push((
                        ((anchor[0] - self.grid.origin[0]) / self.grid.pitch).round() as i32,
                        ((anchor[1] - self.grid.origin[1]) / self.grid.pitch).round() as i32,
                        // Rounding the anchor to a node costs at most one pitch.
                        net_state.terminal_reach[element] + pitch,
                    ));
                } else {
                    exact_points = false;
                    for node in &retained[element - terminal_count].nodes {
                        targets.push((*node, element as u16));
                    }
                }
            }
            if targets.is_empty() {
                break;
            }
            let points = (exact_points && points.len() <= 12).then_some(points);
            let windowed = self.window(net, window_growth);
            let full = (0, 0, self.grid.nx - 1, self.grid.ny - 1);
            // Two-level search: a corridor from the tile graph first, wider
            // on failure, and only then the plain window and the full board.
            let mut planned = None;
            let reroutes = net_state.reroutes;
            let global = self.global.as_ref().filter(|_| !self.cleanup && !hard);
            if let Some(plan) = global {
                // The global plan's corridor, wider for stubborn nets.
                let margin = 1 + reroutes / 4;
                for margin in [margin, margin + 2] {
                    let Some(corridor) = plan.corridor(net as usize, margin) else {
                        break;
                    };
                    scratch.corridor = Some(corridor);
                    planned = self.search(
                        scratch,
                        net,
                        net_state,
                        &tree,
                        &targets,
                        points.as_deref(),
                        false,
                        present,
                        hard,
                        full,
                    );
                    scratch.corridor = None;
                    if planned.is_some() {
                        break;
                    }
                }
            } else if self.config.corridors && !self.cleanup {
                // One tile more every six reroutes. Widened every three,
                // the corridors of the nets the negotiation kept rerouting
                // gave their searches room the paths never used (Sisu and
                // jetson: 73 % of a late iteration's expansions more than
                // three tiles from the path found). The slower growth: on
                // the quick tier 10 % less CPU, unconnected 32 to 30,
                // starved thermals 20 to 15, vias -6 %, no pass lost; in
                // route mode nonSNES SNSP-CPU-01 171 to 98 unconnected at
                // +6 % vias. A cap instead (at most 3 or 5 tiles) left
                // SNSP-CPU-01 unconverged: some nets need the wide detour.
                let margin = (1 + reroutes / 6).min(10);
                for margin in [margin, margin + 3] {
                    let corridor = self.plan_corridor(
                        net,
                        net_state,
                        &tree,
                        &targets,
                        hard,
                        margin,
                        scratch.own_active.then_some(scratch.own_claimed.as_slice()),
                    );
                    let Some(corridor) = corridor else {
                        break;
                    };
                    scratch.corridor = Some(corridor);
                    planned = self.search(
                        scratch,
                        net,
                        net_state,
                        &tree,
                        &targets,
                        points.as_deref(),
                        false,
                        present,
                        hard,
                        full,
                    );
                    scratch.corridor = None;
                    if planned.is_some() {
                        break;
                    }
                }
            }
            // Board-wide fallbacks, unless they keep failing where a
            // corridor was searched first.
            let corridor_tried = global.is_some() || (self.config.corridors && !self.cleanup);
            let fallback = hard || !corridor_tried || net_state.hopeless_searches < 2;
            let path = planned.or_else(|| {
                if !fallback {
                    return None;
                }
                self.search(
                    scratch,
                    net,
                    net_state,
                    &tree,
                    &targets,
                    points.as_deref(),
                    false,
                    present,
                    hard,
                    windowed,
                )
            });
            let path = path.or_else(|| {
                (fallback && windowed != full)
                    .then(|| {
                        self.search(
                            scratch,
                            net,
                            net_state,
                            &tree,
                            &targets,
                            points.as_deref(),
                            false,
                            present,
                            hard,
                            full,
                        )
                    })
                    .flatten()
            });
            let Some(path) = path else {
                if fallback && !hard {
                    net_state.hopeless_searches = net_state.hopeless_searches.saturating_add(1);
                    // The board-wide search reached none of the open
                    // terminals: only what never changes in a negotiation
                    // stops it, so none will be reached in this one. The
                    // net's later searches leave them out; without that,
                    // once the net kept to its corridors (two such
                    // failures), a corridor that missed stopped the whole
                    // net (SNSP-CPU-01's GND with four walled-in pads: 48
                    // of 199 terminals, then frozen as blocked; 178 open).
                    if net_state.unreachable.len() != terminal_count {
                        net_state.unreachable = vec![false; terminal_count];
                    }
                    for (unreachable, reached) in net_state.unreachable.iter_mut().zip(&in_tree) {
                        if !reached {
                            *unreachable = true;
                        }
                    }
                }
                if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                    let open: Vec<usize> = (0..terminal_count)
                        .filter(|element| !in_tree[*element])
                        .collect();
                    eprintln!(
                        "route_net {}: search failed (hard {hard}, fallback {fallback}, corridor tried {corridor_tried}, hopeless {}, cleanup {}), {} of {terminal_count} terminals in the tree, open {:?}",
                        self.board.nets[net as usize].name,
                        net_state.hopeless_searches,
                        self.cleanup,
                        terminal_count - open.len(),
                        &open[..open.len().min(12)]
                    );
                }
                break;
            };
            let first = self.state(path[0]);
            let last = self.state(*path.last().unwrap());
            let start_terminal = scratch
                .tree
                .get(&(first as u32))
                .copied()
                .unwrap_or(NO_TERMINAL);
            let reached = scratch.targets[&(last as u32)] as usize;
            for node in &path[1..] {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    scratch.tree.entry(self.state(*node) as u32)
                {
                    entry.insert(NO_TERMINAL);
                    tree.push(*node);
                }
            }
            absorb(
                scratch,
                net_state,
                &mut tree,
                &mut in_tree,
                component[reached],
            );
            net_state.branches.push(Branch {
                nodes: path,
                start_terminal,
                end_terminal: if reached < terminal_count {
                    reached as u16
                } else {
                    NO_TERMINAL
                },
            });
        }
        let apart = self.route_apart(
            scratch,
            net,
            net_state,
            &component,
            &mut in_tree,
            present,
            hard,
            &absorb,
        );
        let complete = !apart && in_tree.iter().all(|inside| *inside);
        net_state.complete = complete;
        if !hard && !complete {
            net_state.blocked = true;
        }
    }

    /// `Config::apart_trees`: the terminals no path reaches from the net's
    /// tree (`NetState::unreachable`), joined among themselves in trees of
    /// their own, each grown from the first such terminal left by searches
    /// from that tree alone (the main tree's region is not searched
    /// again). Their terminals stay unconnected by the net's count. Returns
    /// whether any terminal was taken into such a tree.
    #[allow(clippy::too_many_arguments)]
    fn route_apart(
        &self,
        scratch: &mut Scratch,
        net: NetId,
        net_state: &mut NetState,
        component: &[usize],
        in_tree: &mut Vec<bool>,
        present: f32,
        hard: bool,
        absorb: &Absorb<'_>,
    ) -> bool {
        let terminal_count = self.board.nets[net as usize].terminals.len();
        if !self.config.apart_trees || net_state.unreachable.len() != terminal_count {
            return false;
        }
        // The groups (of terminals and retained branches) that hold an
        // unreachable terminal.
        let mut apart_group = vec![false; component.len()];
        for (terminal, unreachable) in net_state.unreachable.iter().enumerate() {
            if *unreachable && !in_tree[terminal] {
                apart_group[component[terminal]] = true;
            }
        }
        let full = (0, 0, self.grid.nx - 1, self.grid.ny - 1);
        let mut apart: Vec<usize> = Vec::new();
        while let Some(root) = (0..terminal_count).find(|terminal| {
            !in_tree[*terminal]
                && apart_group[component[*terminal]]
                && !net_state.terminal_nodes[*terminal].is_empty()
        }) {
            scratch.tree.clear();
            let mut tree: Vec<Node> = Vec::new();
            let before: Vec<bool> = in_tree.clone();
            absorb(scratch, net_state, &mut tree, in_tree, component[root]);
            loop {
                let mut targets: Vec<(Node, u16)> = Vec::new();
                for element in 0..component.len() {
                    if in_tree[element] || !apart_group[component[element]] {
                        continue;
                    }
                    if element < terminal_count {
                        for node in &net_state.terminal_nodes[element] {
                            targets.push((*node, element as u16));
                        }
                    }
                }
                if targets.is_empty() {
                    break;
                }
                let Some(path) = self.search(
                    scratch, net, net_state, &tree, &targets, None, false, present, hard, full,
                ) else {
                    break;
                };
                let first = self.state(path[0]);
                let last = self.state(*path.last().unwrap());
                let start_terminal = scratch
                    .tree
                    .get(&(first as u32))
                    .copied()
                    .unwrap_or(NO_TERMINAL);
                let reached = scratch.targets[&(last as u32)] as usize;
                for node in &path[1..] {
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        scratch.tree.entry(self.state(*node) as u32)
                    {
                        entry.insert(NO_TERMINAL);
                        tree.push(*node);
                    }
                }
                absorb(scratch, net_state, &mut tree, in_tree, component[reached]);
                net_state.branches.push(Branch {
                    nodes: path,
                    start_terminal,
                    end_terminal: reached as u16,
                });
            }
            apart.extend(
                (0..terminal_count).filter(|terminal| in_tree[*terminal] && !before[*terminal]),
            );
        }
        for terminal in &apart {
            net_state.connected[*terminal] = false;
        }
        if !apart.is_empty() && std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
            eprintln!(
                "route_net {}: {} terminals no path reaches from the tree joined apart",
                self.board.nets[net as usize].name,
                apart.len()
            );
        }
        !apart.is_empty()
    }

    /// Routing for a net with copper pours: every group of terminals and
    /// retained branches that does not touch a pour is connected to the
    /// nearest free pour node, or to another group on the way.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn connect_to_plane(
        &self,
        scratch: &mut Scratch,
        net: NetId,
        net_state: &mut NetState,
        retained: &[Branch],
        component: &[usize],
        plane_element: usize,
        present: f32,
        hard: bool,
    ) {
        let terminal_count = self.board.nets[net as usize].terminals.len();
        let mut parent: Vec<usize> = component.to_vec();
        let full = (0, 0, self.grid.nx - 1, self.grid.ny - 1);
        let mut failed = vec![false; parent.len()];
        loop {
            // Done when all terminals are one group; the pour is only a
            // means to that end and need not be reached at all.
            let mut roots: Vec<usize> = (0..terminal_count)
                .map(|terminal| find(&mut parent, terminal))
                .collect();
            roots.sort_unstable();
            roots.dedup();
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                eprintln!(
                    "connect_to_plane {}: {} groups, hard {hard}, off-pour terminals {:?}",
                    self.board.nets[net as usize].name,
                    roots.len(),
                    (0..terminal_count)
                        .filter(|terminal| !net_state.on_plane[*terminal])
                        .collect::<Vec<_>>()
                );
            }
            if roots.len() <= 1 {
                break;
            }
            let plane_root = find(&mut parent, plane_element);
            let Some(root) = roots
                .into_iter()
                .find(|root| *root != plane_root && !failed[*root])
            else {
                break;
            };
            scratch.tree.clear();
            let mut sources: Vec<Node> = Vec::new();
            let mut targets: Vec<(Node, u16)> = Vec::new();
            for element in (0..parent.len()).filter(|element| *element != plane_element) {
                let inside = find(&mut parent, element) == root;
                let nodes: Vec<Node> = if element < terminal_count {
                    net_state.terminal_nodes[element].clone()
                } else if element < plane_element {
                    retained[element - terminal_count].nodes.clone()
                } else {
                    // Branches added by this call follow the retained ones.
                    net_state.branches[retained.len() + element - plane_element - 1]
                        .nodes
                        .clone()
                };
                for node in nodes {
                    if inside {
                        if let std::collections::hash_map::Entry::Vacant(entry) =
                            scratch.tree.entry(self.state(node) as u32)
                        {
                            entry.insert(if element < terminal_count {
                                element as u16
                            } else {
                                NO_TERMINAL
                            });
                            sources.push(node);
                        }
                    } else {
                        targets.push((node, element as u16));
                    }
                }
            }
            let Some(path) = self.search(
                scratch, net, net_state, &sources, &targets, None, true, present, hard, full,
            ) else {
                if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                    let restricted = !net_state.plane_target.is_empty();
                    let plane_nodes: usize = if restricted {
                        net_state
                            .plane_target
                            .iter()
                            .map(|m| m.iter().filter(|b| **b).count())
                            .sum()
                    } else {
                        net_state
                            .plane
                            .iter()
                            .map(|m| m.iter().filter(|b| **b).count())
                            .sum()
                    };
                    eprintln!(
                        "  no path: group root {root}, {} sources, {} group targets, plane nodes {plane_nodes} (restricted {restricted}), hard {hard}",
                        sources.len(),
                        targets.len()
                    );
                }
                failed[root] = true;
                continue;
            };
            let first = self.state(path[0]);
            let last = self.state(*path.last().unwrap());
            let start_terminal = scratch
                .tree
                .get(&(first as u32))
                .copied()
                .unwrap_or(NO_TERMINAL);
            let (reached, end_terminal) = if scratch.nodes[last].target_mark == scratch.generation {
                let element = scratch.targets[&(last as u32)] as usize;
                (
                    element,
                    if element < terminal_count {
                        element as u16
                    } else {
                        NO_TERMINAL
                    },
                )
            } else {
                (plane_element, PLANE_TERMINAL)
            };
            let (a, b) = (find(&mut parent, root), find(&mut parent, reached));
            parent[a] = b;
            net_state.branches.push(Branch {
                nodes: path,
                start_terminal,
                end_terminal,
            });
            // The new branch is an element of its own from now on.
            parent.push(b);
            failed.push(false);
        }
        let mut votes: HashMap<usize, usize> = HashMap::new();
        for terminal in 0..terminal_count {
            *votes.entry(find(&mut parent, terminal)).or_default() += 1;
        }
        let main = votes
            .into_iter()
            .max_by_key(|(root, count)| (*count, usize::MAX - *root))
            .map_or(0, |(root, _)| root);
        let mut complete = true;
        for terminal in 0..terminal_count {
            let connected = find(&mut parent, terminal) == main;
            net_state.connected[terminal] = connected;
            complete &= connected;
        }
        net_state.complete = complete;
        if !hard && !complete {
            net_state.blocked = true;
        }
    }
}
