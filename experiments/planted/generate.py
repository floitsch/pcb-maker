#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Generates boards that are routable by construction: real footprints, a
netlist, a planted routing (clearances by construction), and for the maze
approach netless blockers in the space the planted routes leave.

    experiments/planted/generate.py <out-dir> --mode maze|grow [knobs] [--drc]

Writes <out-dir>/<board-id>.kicad_pcb, .kicad_pro and planted.json (every
knob and the planted solution's statistics).

maze: parts scattered (passives clustered around ICs), nets by proximity,
      every net planted with waypoint detours and layer changes, then
      netless blockers dropped into the free space on the straight lines of
      the detoured connections and elsewhere (blocker density), keeping
      `corridor` free tracks beside every planted route.
grow: a nucleus IC, then parts added one at a time next to the pads they
      connect to, into the space the routes so far leave, and their
      connections planted at once; the parts themselves are the blockers.
      Compactness (how close a part may come to copper and courtyards) is
      the difficulty."""

import argparse
import json
import math
import random
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import grid as G  # noqa: E402
import numpy as np  # noqa: E402
from planted import ORIGIN, Library, PlantBoard, Rules  # noqa: E402

MIX = {"ic": 0.06, "small": 0.12, "passive": 0.70, "connector": 0.07, "tht": 0.05}


def choose(rng, names, weights=None):
    return rng.choices(names, weights=weights)[0] if weights else rng.choice(names)


def pad_terminal(group):
    """The pad of a same-number group that carries the routing (largest
    copper)."""
    copper = [p for p in group if p.copper and p.layers]
    if not copper:
        return None
    return max(copper, key=lambda p: (p.shape.box[2] - p.shape.box[0]) * (p.shape.box[3] - p.shape.box[1]))


class Generator:
    def __init__(self, args):
        self.args = args
        self.rng = random.Random(args.seed)
        self.library = Library()
        rules = Rules(track=args.track, clearance=args.clearance, via_diameter=args.via_diameter,
                      via_drill=args.via_drill)
        self.rules = rules
        area = args.parts * args.area_per_part
        self.width = round(math.sqrt(area * args.aspect), 1)
        self.height = round(area / self.width, 1)
        self.pb = PlantBoard(self.library, self.width, self.height, args.layers, rules, pitch=args.pitch,
                             seed=args.seed)
        self.log = {"failed_pads": 0, "dropped_nets": 0, "placement_failures": 0}
        self.power_reached = {}

    def register_planes(self, gnd, vcc):
        """`--planes N` on four or more layers: the first inner layer is a
        GND pour, the second (N = 2) a +3V3 pour; signals stay off them."""
        pb = self.pb
        if pb.layer_count >= 4 and self.args.planes >= 1:
            pb.planes[1] = gnd
        if pb.layer_count >= 4 and self.args.planes >= 2:
            pb.planes[2] = vcc

    # Placement -----------------------------------------------------------
    def part_names(self):
        rng = self.rng
        lib = self.library
        mix = dict(MIX)
        if self.args.mix:
            text = self.args.mix
            if text.lstrip().startswith("{"):
                mix.update(json.loads(text))
            else:
                mix.update({k: float(v) for k, v in (item.split("=") for item in text.split(","))})
        cats = list(mix)
        names = []
        for _ in range(self.args.parts):
            cat = rng.choices(cats, weights=[mix[c] for c in cats])[0]
            names.append(rng.choice(lib.names(cat)))
        order = {"ic": 0, "tht": 1, "connector": 2, "small": 3, "passive": 4}
        names.sort(key=lambda n: order[lib.index[n]["category"]])
        return names

    def scatter(self):
        rng = self.rng
        pb = self.pb
        x0, y0 = ORIGIN
        anchors = []
        for name in self.part_names():
            category = self.library.index[name]["category"]
            placed = None
            for attempt in range(300):
                if category in ("passive", "small") and anchors and rng.random() < 0.8:
                    ax, ay = rng.choice(anchors)
                    x = ax + rng.gauss(0, self.args.cluster)
                    y = ay + rng.gauss(0, self.args.cluster)
                else:
                    x = x0 + rng.uniform(2, self.width - 2)
                    y = y0 + rng.uniform(2, self.height - 2)
                x = round(x / 0.1) * 0.1
                y = round(y / 0.1) * 0.1
                placed = pb.try_place(name, x, y, rng.choice([0, 90, 180, 270]))
                if placed:
                    break
            if placed is None:
                self.log["placement_failures"] += 1
                continue
            if category in ("ic", "small", "connector", "tht"):
                anchors.append((x, y))

    # Netlist ---------------------------------------------------------------
    def assign_nets(self):
        """Power pins to GND / VCC, the rest into signal nets by proximity."""
        rng = self.rng
        pb = self.pb
        a = self.args
        gnd = pb.add_net("GND")
        vcc = pb.add_net("+3V3")
        self.register_planes(gnd, vcc)
        terminals = []  # (part, number, pads)
        for part in pb.parts:
            for number, group in part.groups().items():
                if pad_terminal(group) is None:
                    continue
                terminals.append((part, number, group))
        signals = []
        for part, number, group in terminals:
            cat = part.category
            n_pins = len(part.groups())
            size = max(p.shape.box[2] - p.shape.box[0] for p in group) * max(p.shape.box[3] - p.shape.box[1] for p in group)
            if cat in ("ic", "small") and size > 2.0:
                net = gnd  # exposed pads and tabs
            elif cat == "passive":
                net = None
                if number == "2" and rng.random() < a.passive_power:
                    net = gnd if rng.random() < 0.6 else vcc
            elif cat == "hole":
                continue
            else:
                u = rng.random()
                if u < a.power_fraction * 0.6:
                    net = gnd
                elif u < a.power_fraction:
                    net = vcc
                elif u < a.power_fraction + a.nc_fraction and n_pins > 4:
                    continue
                else:
                    net = None
            if net is None:
                signals.append((part, number, group))
            else:
                for p in group:
                    p.net = net
        # Signal nets: each unassigned terminal starts a net and pulls in
        # one to three nearby terminals of other parts.
        rng.shuffle(signals)
        free = set(range(len(signals)))
        centre = [pad_terminal(g).centre for _, _, g in signals]
        count = 0
        for i in range(len(signals)):
            if i not in free:
                continue
            free.discard(i)
            k = rng.choices([1, 2, 3, 4, 5, 6, 7], weights=a.net_sizes)[0]
            members = [i]
            for _ in range(k):
                candidates = [j for j in free if all(signals[j][0] is not signals[m][0] for m in members)]
                if not candidates:
                    break
                weights = [math.exp(-min(math.dist(centre[j], centre[m]) for m in members) / a.locality)
                           for j in candidates]
                if sum(weights) <= 0:
                    break
                j = rng.choices(candidates, weights=weights)[0]
                free.discard(j)
                members.append(j)
            if len(members) < 2:
                continue
            count += 1
            net = pb.add_net(f"N{count}")
            for m in members:
                for p in signals[m][2]:
                    p.net = net

    # Routing ---------------------------------------------------------------
    def winder(self, wind_prob, detour_max):
        rng = self.rng
        x0, y0 = ORIGIN

        def wind(a, b):
            d = math.dist(a, b)
            if d < self.args.min_wind_length or rng.random() > wind_prob:
                return None
            rho = rng.uniform(1.15, detour_max)
            h = d / 2 * math.sqrt(rho * rho - 1)
            t = rng.uniform(0.3, 0.7)
            nx, ny = -(b[1] - a[1]) / d, (b[0] - a[0]) / d
            side = rng.choice([-1, 1])
            px = a[0] + t * (b[0] - a[0]) + side * h * nx
            py = a[1] + t * (b[1] - a[1]) + side * h * ny
            px = min(max(px, x0 + 1), x0 + self.width - 1)
            py = min(max(py, y0 + 1), y0 + self.height - 1)
            return (px, py)

        return wind

    def net_pads(self, net):
        groups = {}
        for part in self.pb.parts:
            for pad in part.pads:
                if pad.net == net:
                    groups.setdefault((id(part), pad.number), []).append(pad)
        return [pad_terminal(g) for g in groups.values() if pad_terminal(g) is not None]

    def route_one(self, net, wind):
        pb = self.pb
        a = self.args
        rng = self.rng
        pads = self.net_pads(net)
        if len(pads) < 2:
            return pads, []
        rng.shuffle(pads)
        if net in pb.planes.values():
            connected, failed = [], []
            for pad in pads:
                (connected if pb.connect_to_plane(net, pad, a.via_cost / a.pitch) else failed).append(pad)
            return connected, failed
        layers = pb.signal_layers()
        common = set(layers)
        for p in pads:
            common &= set(p.layers)
        if rng.random() < a.one_layer and common:
            allowed = [min(common)] if 0 in common else [sorted(common)[0]]
            cost = [1.0] * pb.layer_count
        else:
            allowed = layers
            cost = [1.0] * pb.layer_count
            cost[0] = rng.uniform(1.0, a.front_cost)
        connected, failed = pb.route_net(net, pads, wind=wind, layer_cost=cost, allowed_layers=allowed,
                                         via_cost=a.via_cost / a.pitch)
        return connected, failed

    def route_all(self, wind):
        pb = self.pb
        from stats import mst_length
        nets = list(range(len(pb.nets)))
        # Short nets first (a sequential router's usual order), with some
        # shuffling; the power trees last.
        span = {n: mst_length([p.centre for p in self.net_pads(n)]) * self.rng.uniform(0.7, 1.4) for n in nets}
        nets.sort(key=lambda n: (pb.nets[n] in ("GND", "+3V3"), span[n]))
        for net in nets:
            connected, failed = self.route_one(net, wind)
            self.settle(net, connected, failed)

    def settle(self, net, connected, failed):
        """Pads that the planting could not reach leave the net (they stay
        stamped as the net's copper, so clearances hold either way)."""
        if failed:
            self.log["failed_pads"] += len(failed)
        dead = set(id(p.part) * 0 + id(p) for p in failed)
        groups = {(id(p.part), p.number) for p in failed}
        for part in self.pb.parts:
            for pad in part.pads:
                if pad.net == net and (id(part), pad.number) in groups:
                    pad.net = None
        if len(connected) < 2:
            self.log["dropped_nets"] += 1
            for part in self.pb.parts:
                for pad in part.pads:
                    if pad.net == net:
                        pad.net = None

    def netlist_summary(self):
        pb = self.pb
        terminals = {}
        total = 0
        for part in pb.parts:
            if part in pb.blockers:
                continue
            for number, group in part.groups().items():
                if pad_terminal(group) is None:
                    continue
                total += 1
                net = group[0].net
                if net is not None:
                    terminals.setdefault(net, 0)
                    terminals[net] += 1
        nets = {n: c for n, c in terminals.items() if c >= 2}
        return {"terminals": total, "terminals_on_nets": sum(nets.values()), "nets": len(nets),
                "connections": sum(c - 1 for c in nets.values())}

    # Blockers --------------------------------------------------------------
    def drop_blockers(self):
        pb = self.pb
        rng = self.rng
        a = self.args
        g = pb.grid
        names = a.blockers.split(",")
        margin = a.corridor * (a.track + a.clearance)
        placed = 0
        # On the straight lines of detoured connections first.
        lines = [(c["pad_a"], c["pad_b"]) for c in pb.connections if c["length_mm"] > 1.25 * math.dist(c["pad_a"], c["pad_b"]) and math.dist(c["pad_a"], c["pad_b"]) > 2]
        attempts = int(a.blocker_density * self.width * self.height / 4)
        for k in range(attempts):
            if lines and k < attempts * 0.6:
                (ax, ay), (bx, by) = rng.choice(lines)
                t = rng.uniform(0.15, 0.85)
                x, y = ax + t * (bx - ax), ay + t * (by - ay)
            else:
                # A random node far from copper and courtyards.
                free = ~g.court[0]
                for layer in range(pb.layer_count):
                    free &= g.trk[layer] == G.FREE
                iy, ix = np.nonzero(free)
                if len(ix) == 0:
                    break
                k2 = rng.randrange(len(ix))
                x, y = g.x(int(ix[k2])), g.y(int(iy[k2]))
            x = round(x / 0.1) * 0.1
            y = round(y / 0.1) * 0.1
            name = rng.choice(names)
            if pb.try_blocker(name, x, y, rng.choice([0, 90]), margin):
                placed += 1
        return placed

    # Grow ------------------------------------------------------------------
    def grow(self, wind):
        """A nucleus IC, then parts added one at a time next to a terminal
        waiting for a partner, into the space the routes so far leave; the
        new part's connections (to its partners and to the power trees)
        are planted at once, and the addition is undone when one fails."""
        pb = self.pb
        rng = self.rng
        names = self.part_names()
        self.gnd = pb.add_net("GND")
        self.vcc = pb.add_net("+3V3")
        self.register_planes(self.gnd, self.vcc)
        x0, y0 = ORIGIN
        self.pending = []
        ics = [n for n in names if self.library.index[n]["category"] == "ic"]
        nucleus = ics[0] if ics else names[0]
        names.remove(nucleus)
        rng.shuffle(names)
        # Larger parts tend to come first, as a designer places them.
        weight = {"ic": 0, "tht": 1, "connector": 1, "small": 2, "passive": 3}
        names.sort(key=lambda n: weight[self.library.index[n]["category"]] + rng.uniform(0, 2.5))
        self.add_part(nucleus, (x0 + self.width / 2, y0 + self.height / 2), [], wind, tries=1)
        queue = list(names)
        while queue:
            # Few terminals waiting: an IC or connector next, as a designer
            # would add the next block of the circuit.
            if len(self.pending) < 4:
                big = [n for n in queue if self.library.index[n]["category"] in ("ic", "connector", "tht", "small")]
                name = big[0] if big else queue[0]
            else:
                name = queue[0]
            queue.remove(name)
            if not self.pending:
                # A new nucleus anywhere free.
                spot = (x0 + rng.uniform(3, self.width - 3), y0 + rng.uniform(3, self.height - 3))
                if not self.add_part(name, spot, [], wind):
                    self.log["placement_failures"] += 1
                continue
            for retry in range(self.args.grow_retries):
                cat = self.library.index[name]["category"]
                first = rng.choice(self.pending)
                k = 1 if cat in ("passive", "small") else rng.choice([1, 2, 3, 4])
                partners = [first]
                near = sorted(self.pending, key=lambda t: math.dist(t.centre, first.centre))
                for t in near[1:k]:
                    if math.dist(t.centre, first.centre) < 3 * self.args.cluster:
                        partners.append(t)
                if self.add_part(name, first.centre, partners, wind):
                    break
            else:
                # No room near any partner: the part starts a new cluster
                # wherever the board still has room.
                if not self.add_part(name, self.free_spot(), [], wind):
                    self.log["placement_failures"] += 1
        return self.pending

    def free_spot(self):
        g = self.pb.grid
        free = ~g.court[0]
        for layer in range(self.pb.layer_count):
            free &= g.trk[layer] == G.FREE
        iy, ix = np.nonzero(free)
        if len(ix) == 0:
            return (ORIGIN[0] + self.width / 2, ORIGIN[1] + self.height / 2)
        k = self.rng.randrange(len(ix))
        return (g.x(int(ix[k])), g.y(int(iy[k])))

    def add_part(self, name, anchor, partners, wind, tries=None):
        pb = self.pb
        rng = self.rng
        a = self.args
        cat = self.library.index[name]["category"]
        margin = a.corridor * (a.track + a.clearance)
        part = None
        tries = tries or a.grow_attempts
        state = self.save()
        for attempt in range(tries):
            spread = a.cluster * (1 + attempt / 30)
            x = round((anchor[0] + rng.gauss(0, spread)) / 0.1) * 0.1
            y = round((anchor[1] + rng.gauss(0, spread)) / 0.1) * 0.1
            if tries == 1:
                x, y = round(anchor[0], 1), round(anchor[1], 1)
            part = pb.try_place(name, x, y, rng.choice([0, 90, 180, 270]), nets={}, margin=margin)
            if part:
                break
        if part is None:
            self.log["no_spot"] = self.log.get("no_spot", 0) + 1
            return False
        groups = [(num, grp) for num, grp in part.groups().items() if pad_terminal(grp) is not None]
        plans = []
        for partner in partners:
            if not groups:
                break
            num, grp = min(groups, key=lambda t: math.dist(pad_terminal(t[1]).centre, partner.centre))
            groups.remove((num, grp))
            plans.append((partner, grp))
        new_pending = []
        for num, grp in groups:
            size = max(p.shape.box[2] - p.shape.box[0] for p in grp) * \
                max(p.shape.box[3] - p.shape.box[1] for p in grp)
            u = rng.random()
            if cat in ("ic", "small") and size > 2.0:
                net = self.gnd
            elif cat == "passive":
                net = (self.gnd if rng.random() < 0.6 else self.vcc) if u < a.passive_power else None
            elif u < a.power_fraction * 0.6:
                net = self.gnd
            elif u < a.power_fraction:
                net = self.vcc
            elif u < a.power_fraction + a.nc_fraction and len(part.groups()) > 4:
                net = pb.add_net(f"NC{len(pb.nets)}")
            else:
                net = None
            if net is None:
                net = pb.add_net(f"N{len(pb.nets)}")
                new_pending.append(pad_terminal(grp))
            for p in grp:
                p.net = net
        for partner, grp in plans:
            for p in grp:
                p.net = partner.net
        for num, grp in part.groups().items():
            if pad_terminal(grp) is None:
                for p in grp:
                    p.net = None
        pb.stamp_part(part)
        ok = True
        for partner, grp in plans:
            if self.join(partner.net, [partner], pad_terminal(grp), wind) is None:
                ok = False
                break
        if ok:
            for net in (self.gnd, self.vcc):
                mine = [pad_terminal(grp) for num, grp in part.groups().items()
                        if pad_terminal(grp) is not None and grp[0].net == net]
                for pad in mine:
                    others = list(self.power_reached.get(net, []))
                    if not others and net not in pb.planes.values():
                        self.power_reached.setdefault(net, []).append(pad)
                        continue
                    if self.join(net, others, pad, wind) is None:
                        ok = False
                        break
                    self.power_reached.setdefault(net, []).append(pad)
                if not ok:
                    break
        if not ok:
            self.restore(state, part)
            self.log["unroutable"] = self.log.get("unroutable", 0) + 1
            return False
        for partner, grp in plans:
            # A net may take more pads: its terminal stays waiting.
            if partner in self.pending and rng.random() > a.multi_pad:
                self.pending.remove(partner)
        self.pending.extend(new_pending)
        return True

    def join(self, net, reached, pad, wind):
        """Connects `pad` to the copper of `net` (its tree, or the pads in
        `reached` when it has none yet)."""
        pb = self.pb
        a = self.args
        if net in pb.planes.values():
            return pb.connect_to_plane(net, pad, a.via_cost / a.pitch)
        tree = pb.trees.setdefault(net, set())
        if not tree:
            for p in reached:
                pb.tree_add_pad(net, tree, p)
        cost = [1.0] * pb.layer_count
        cost[0] = self.rng.uniform(1.0, a.front_cost)
        allowed = pb.signal_layers()
        if self.rng.random() < a.one_layer:
            common = set(allowed) & set(pad.layers)
            for p in reached:
                common &= set(p.layers)
            if common:
                allowed = [min(common)]
        return pb.connect(net, tree, pad, wind, cost, a.via_cost / a.pitch, allowed)

    def save(self):
        pb = self.pb
        g = pb.grid
        return {"arrays": (g.trk.copy(), g.via.copy(), g.court.copy(), g.copper.copy()),
                "connections": len(pb.connections), "nets": len(pb.nets),
                "trees": {n: set(t) for n, t in pb.trees.items()},
                "stubs": set(pb.stubs), "pad_states": dict(pb.pad_states),
                "power": {n: list(v) for n, v in self.power_reached.items()}}

    def restore(self, state, part):
        pb = self.pb
        g = pb.grid
        g.trk[:], g.via[:], g.court[:], g.copper[:] = state["arrays"]
        for record in pb.connections[state["connections"]:]:
            edges, vias = record["added"]
            copper = pb.copper[record["net"]]
            for e in edges:
                copper["edges"].discard(e)
            for v in vias:
                copper["vias"].discard(v)
        del pb.connections[state["connections"]:]
        for net in range(state["nets"], len(pb.nets)):
            del pb.copper[net]
        del pb.nets[state["nets"]:]
        pb.trees = state["trees"]
        pb.stubs = state["stubs"]
        pb.pad_states = state["pad_states"]
        self.power_reached = state["power"]
        pb.parts.remove(part)
        pb.board.Remove(part.fp)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out")
    parser.add_argument("--mode", choices=["maze", "grow"], default="maze")
    parser.add_argument("--board-id", default="planted")
    parser.add_argument("--level", default="", help="a difficulty label recorded with the knobs")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--layers", type=int, default=2)
    parser.add_argument("--parts", type=int, default=40)
    parser.add_argument("--area-per-part", type=float, default=40.0, help="mm^2 of board per part")
    parser.add_argument("--aspect", type=float, default=1.3)
    parser.add_argument("--mix", help="category mix overrides: passive=0.8,ic=0.05,... (or JSON)")
    parser.add_argument("--cluster", type=float, default=3.0, help="mm, spread of passives around anchors")
    parser.add_argument("--track", type=float, default=0.2)
    parser.add_argument("--clearance", type=float, default=0.15)
    parser.add_argument("--via-diameter", type=float, default=0.6)
    parser.add_argument("--via-drill", type=float, default=0.3)
    parser.add_argument("--pitch", type=float, default=0.1)
    parser.add_argument("--power-fraction", type=float, default=0.15)
    parser.add_argument("--passive-power", type=float, default=0.5)
    parser.add_argument("--nc-fraction", type=float, default=0.15)
    parser.add_argument("--locality", type=float, default=6.0, help="mm, signal net partner distance scale")
    parser.add_argument("--wind", type=float, default=0.15, help="share of connections given a detour waypoint")
    parser.add_argument("--detour-max", type=float, default=1.6)
    parser.add_argument("--net-sizes", type=lambda t: [float(v) for v in t.split(",")],
                        default=[0.5, 0.2, 0.1, 0.07, 0.06, 0.04, 0.03],
                        help="weights of signal nets with 2, 3, ... 8 pads")
    parser.add_argument("--min-wind-length", type=float, default=3.0)
    parser.add_argument("--one-layer", type=float, default=0.6, help="share of nets planted on one layer")
    parser.add_argument("--front-cost", type=float, default=1.25, help="max cost factor of F.Cu for multi-layer nets")
    parser.add_argument("--via-cost", type=float, default=4.0, help="mm of track a via is worth")
    parser.add_argument("--blocker-density", type=float, default=0.0, help="blocker attempts per 4 mm^2")
    parser.add_argument("--blockers", default="TestPoint_THTPad_D1.0mm_Drill0.5mm,TestPoint_THTPad_D1.0mm_Drill0.5mm,MountingHole_2.2mm_M2_DIN965,PinHeader_1x02_P2.54mm_Vertical,TestPoint_Pad_D1.0mm,C_0805_2012Metric,R_0603_1608Metric")
    parser.add_argument("--corridor", type=float, default=1.0, help="free tracks kept beside a planted route by blockers")
    parser.add_argument("--grow-attempts", type=int, default=60)
    parser.add_argument("--grow-retries", type=int, default=6)
    parser.add_argument("--multi-pad", type=float, default=0.45, help="grow: chance a joined net takes further pads")
    parser.add_argument("--planes", type=int, default=2, help="inner plane pours on 4+ layers (0, 1 GND, 2 GND and +3V3)")
    parser.add_argument("--drc", action="store_true")
    args = parser.parse_args()

    started = time.process_time()
    gen = Generator(args)
    pb = gen.pb
    wind = gen.winder(args.wind, args.detour_max)
    pending = []
    if args.mode == "maze":
        gen.scatter()
        gen.assign_nets()
        for part in pb.parts:
            pb.stamp_part(part)
        gen.route_all(wind)
        blockers = gen.drop_blockers() if args.blocker_density > 0 else 0
    else:
        pending = gen.grow(wind)
        blockers = gen.drop_blockers() if args.blocker_density > 0 else 0
    out = Path(args.out)
    totals = pb.write(out, args.board_id)
    netlist = gen.netlist_summary()
    record = {"knobs": vars(args), "rules": gen.rules.as_dict(), "width_mm": gen.width, "height_mm": gen.height,
              "parts": len(pb.parts) - len(pb.blockers), "blockers": len(pb.blockers), "log": gen.log,
              "planted": totals, "planted_connections": len(pb.connections), "netlist": netlist,
              "generate_cpu_seconds": round(time.process_time() - started, 1)}
    from stats import board_stats
    statistics = board_stats(out / f"{args.board_id}.kicad_pcb", include_nets=False)
    record["statistics"] = {k: v for k, v in statistics.items() if k != "board"}
    (out / "planted.json").write_text(json.dumps(record, indent=1))
    if args.drc:
        report = out / "planted-drc.json"
        subprocess.run(["kicad-cli", "pcb", "drc", "--format", "json", "--severity-all", "--refill-zones",
                        "--output", str(report), str(out / f"{args.board_id}.kicad_pcb")],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        drc = json.loads(report.read_text())
        counts = {}
        for v in drc.get("violations", []):
            counts[v["type"]] = counts.get(v["type"], 0) + 1
        record["drc"] = {"violations": counts, "unconnected": len(drc.get("unconnected_items", []))}
        (out / "planted.json").write_text(json.dumps(record, indent=1))
    print(json.dumps({k: record[k] for k in record if k != "knobs"}))


if __name__ == "__main__":
    main()
