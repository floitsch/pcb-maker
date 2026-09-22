// Copyright (C) 2026 Toit contributors.
//
// Routes a Specctra DSN (as exported by KiCad and rule-translated by the
// pcb-maker pipeline) with tscircuit's capacity autorouter and writes a
// Specctra session that KiCad imports. Usage:
//
//   node route_dsn.mjs input.dsn output.ses report.json [timeout-seconds]
//
// Everything stays in DSN coordinates (micrometres, y up); KiCad's session
// importer maps them back. Pads become rectangular obstacles inflated by
// the clearance, connections are the nets' pad centres.

import { readFileSync, writeFileSync } from "node:fs";
import { AutoroutingPipelineSolver } from "@tscircuit/capacity-autorouter";

// --- Minimal S-expression reader (keeps atoms as strings).
function parse(text) {
  const tokens = text.match(/"(?:[^"\\]|\\.)*"|[^\s()"]+|[()]/g);
  const stack = [[]];
  for (const token of tokens) {
    if (token === "(") stack.push([]);
    else if (token === ")") {
      const list = stack.pop();
      stack[stack.length - 1].push(list);
    } else stack[stack.length - 1].push(token.replace(/^"|"$/g, ""));
  }
  return stack[0][0];
}
const head = (list) => (Array.isArray(list) ? list[0] : undefined);
const child = (list, name) => list.find((item) => head(item) === name);
const children = (list, name) => list.filter((item) => head(item) === name);
const num = (value) => Number(value);

const [dsnPath, sesPath, reportPath, timeoutArg] = process.argv.slice(2);
const timeoutSeconds = Number(timeoutArg ?? "1500");
const text = readFileSync(dsnPath, "utf8");
// Specctra's quote declaration is not a quoted S-expression.
const pcb = parse(text.replace('(string_quote ")', "(string_quote dq)"));
const structure = child(pcb, "structure");
const library = child(pcb, "library");
const network = child(pcb, "network");
const placement = child(pcb, "placement");
const resolution = child(pcb, "resolution");
const unitsPerMm = resolution[1] === "um" ? 1000 : resolution[1] === "mm" ? 1 : 1000;
const toMm = (value) => num(value) / unitsPerMm;

const layerNames = children(structure, "layer").map((layer) => layer[1]);
const layerMap = new Map(); // DSN layer -> tscircuit layer
layerNames.forEach((name, index) => {
  layerMap.set(name, index === 0 ? "top" : index === layerNames.length - 1 ? "bottom" : `inner${index}`);
});
const backMap = new Map([...layerMap].map(([k, v]) => [v, k]));

// Board bounds from the boundary path.
const boundary = child(structure, "boundary");
const boundaryPath = child(boundary, "path");
const boundaryPoints = [];
for (let index = 3; index + 1 < boundaryPath.length; index += 2) {
  boundaryPoints.push({ x: toMm(boundaryPath[index]), y: toMm(boundaryPath[index + 1]) });
}
const bounds = {
  minX: Math.min(...boundaryPoints.map((p) => p.x)),
  maxX: Math.max(...boundaryPoints.map((p) => p.x)),
  minY: Math.min(...boundaryPoints.map((p) => p.y)),
  maxY: Math.max(...boundaryPoints.map((p) => p.y)),
};

// Rules: width and clearance of the default class, via padstack geometry.
const rule = child(structure, "rule");
let traceWidth = toMm(child(rule, "width")[1]);
let clearance = toMm(child(rule, "clearance")[1]);
const viaName = child(structure, "via")?.[1];
const viaPadstack = children(library, "padstack").find((p) => p[1] === viaName);
const viaMatch = /_(\d+(?:\.\d+)?):(\d+(?:\.\d+)?)_um/.exec(viaName ?? "");
const viaDiameterMm = viaMatch ? Number(viaMatch[1]) / 1000 : 0.6;
const viaDrillMm = viaMatch ? Number(viaMatch[2]) / 1000 : 0.3;
const viaShapes = viaPadstack
  ? children(viaPadstack, "shape").map((shape) => {
      const g = shape[1];
      return `        (shape\n          (circle ${g[1]} ${Math.round(toMm(g[2]) * unitsPerMm * Number(resolution[2] ?? "1"))} 0 0)\n        )`;
    })
  : [];

// Padstacks: bounding box per layer.
const padstacks = new Map();
for (const padstack of children(library, "padstack")) {
  const layers = new Map();
  for (const shape of children(padstack, "shape")) {
    const geometry = shape[1];
    const kind = head(geometry);
    const layer = geometry[1];
    let box;
    if (kind === "circle") {
      const r = toMm(geometry[2]) / 2;
      const cx = geometry.length > 4 ? toMm(geometry[3]) : 0;
      const cy = geometry.length > 4 ? toMm(geometry[4]) : 0;
      box = { minX: cx - r, maxX: cx + r, minY: cy - r, maxY: cy + r };
    } else if (kind === "rect") {
      const [x1, y1, x2, y2] = geometry.slice(2, 6).map(toMm);
      box = { minX: Math.min(x1, x2), maxX: Math.max(x1, x2), minY: Math.min(y1, y2), maxY: Math.max(y1, y2) };
    } else if (kind === "polygon") {
      const coords = geometry.slice(3).map(toMm);
      const xs = coords.filter((_, i) => i % 2 === 0);
      const ys = coords.filter((_, i) => i % 2 === 1);
      box = { minX: Math.min(...xs), maxX: Math.max(...xs), minY: Math.min(...ys), maxY: Math.max(...ys) };
    } else if (kind === "path") {
      const half = toMm(geometry[2]) / 2;
      const coords = geometry.slice(3).map(toMm);
      const xs = coords.filter((_, i) => i % 2 === 0);
      const ys = coords.filter((_, i) => i % 2 === 1);
      box = { minX: Math.min(...xs) - half, maxX: Math.max(...xs) + half, minY: Math.min(...ys) - half, maxY: Math.max(...ys) + half };
    } else continue;
    if (layerMap.has(layer)) layers.set(layerMap.get(layer), box);
    else if (layer === "signal" || layer === "pcb") for (const tl of layerMap.values()) layers.set(tl, box);
  }
  padstacks.set(padstack[1], layers);
}

// Images: pins with padstack, number and local position.
const images = new Map();
for (const image of children(library, "image")) {
  const pins = children(image, "pin").map((pin) => {
    let index = 2;
    if (Array.isArray(pin[2]) && head(pin[2]) === "rotate") index = 3;
    return { padstack: pin[1], number: pin[index], x: toMm(pin[index + 1]), y: toMm(pin[index + 2]), rotation: index === 3 ? num(pin[2][1]) : 0 };
  });
  images.set(image[1], pins);
}

// Net membership by "REF-PIN".
const netOfPin = new Map();
const netPins = new Map();
for (const net of children(network, "net")) {
  const pins = child(net, "pins")?.slice(1) ?? [];
  netPins.set(net[1], pins);
  for (const pin of pins) netOfPin.set(pin, net[1]);
}

// Placed pads -> obstacles and connection points.
const rotate = (x, y, degrees) => {
  const r = (degrees * Math.PI) / 180;
  return { x: x * Math.cos(r) - y * Math.sin(r), y: x * Math.sin(r) + y * Math.cos(r) };
};
const obstacles = [];
const padPositions = new Map(); // "REF-PIN" -> {x,y,layers}
for (const component of children(placement, "component")) {
  const pins = images.get(component[1]);
  if (!pins) continue;
  for (const place of children(component, "place")) {
    const [ref, px, py, side, angle] = [place[1], toMm(place[2]), toMm(place[3]), place[4], num(place[5] ?? 0)];
    // KiCad numbers repeated pad numbers REF-N, REF-N@1, REF-N@2, ...
    const seen = new Map();
    for (const pin of pins) {
      const occurrence = seen.get(pin.number) ?? 0;
      seen.set(pin.number, occurrence + 1);
      const key = occurrence === 0 ? `${ref}-${pin.number}` : `${ref}-${pin.number}@${occurrence}`;
      const local = side === "back" ? { x: -pin.x, y: pin.y } : { x: pin.x, y: pin.y };
      const rotated = rotate(local.x, local.y, side === "back" ? -angle : angle);
      const cx = px + rotated.x;
      const cy = py + rotated.y;
      const layers = padstacks.get(pin.padstack) ?? new Map();
      const net = netOfPin.get(key);
      let padLayers = [...layers.keys()];
      if (side === "back") padLayers = padLayers.map((l) => (l === "top" ? "bottom" : l === "bottom" ? "top" : l));
      let union = null;
      for (const box of layers.values()) {
        const b = side === "back" ? { minX: -box.maxX, maxX: -box.minX, minY: box.minY, maxY: box.maxY } : box;
        union = union
          ? { minX: Math.min(union.minX, b.minX), maxX: Math.max(union.maxX, b.maxX), minY: Math.min(union.minY, b.minY), maxY: Math.max(union.maxY, b.maxY) }
          : { ...b };
      }
      if (!union) continue;
      const width = union.maxX - union.minX;
      const height = union.maxY - union.minY;
      const centerLocal = rotate((union.minX + union.maxX) / 2, (union.minY + union.maxY) / 2, side === "back" ? -angle : angle);
      const total = ((side === "back" ? -angle : angle) + pin.rotation) % 360;
      obstacles.push({
        type: "rect",
        layers: padLayers,
        center: { x: cx + centerLocal.x - rotated.x + rotated.x, y: cy + centerLocal.y - rotated.y + rotated.y },
        width,
        height,
        ccwRotationDegrees: total,
        connectedTo: net ? [net] : [],
      });
      padPositions.set(key, { x: cx, y: cy, layers: padLayers });
    }
  }
}
// Keepouts and existing copper are not carried (the cold boards have none).

const connections = [];
for (const [net, pins] of netPins) {
  const points = [];
  for (const pin of pins) {
    const at = padPositions.get(pin);
    if (!at) continue;
    if (points.some((p) => Math.abs(p.x - at.x) < 1e-6 && Math.abs(p.y - at.y) < 1e-6)) continue;
    points.push(at.layers.length > 1 ? { x: at.x, y: at.y, layers: at.layers } : { x: at.x, y: at.y, layer: at.layers[0] ?? "top" });
  }
  if (points.length >= 2) connections.push({ name: net, pointsToConnect: points });
}

// The autorouter keeps trace centrelines minTraceWidth apart; the real
// spacing is width plus clearance. Copper is written at the true width.
// The solver keeps trace centrelines minTraceWidth apart and its own
// clearance fields off pads, vias and the edge; pass the board's rules.
const srj = {
  layerCount: layerNames.length,
  // No trace-to-trace clearance field exists; the solver spaces centrelines
  // by minTraceWidth, so the width plus clearance is the honest value.
  minTraceWidth: traceWidth + clearance,
  nominalTraceWidth: traceWidth,
  defaultObstacleMargin: clearance,
  minTraceToPadEdgeClearance: clearance,
  minViaEdgeToPadEdgeClearance: clearance,
  minBoardEdgeClearance: clearance,
  minViaDiameter: viaDiameterMm,
  minViaHoleDiameter: viaDrillMm,
  obstacles,
  connections,
  bounds,
};
const started = Date.now();
const solver = new AutoroutingPipelineSolver(srj);
let timedOut = false;
while (!solver.solved && !solver.failed) {
  solver.step();
  if ((Date.now() - started) / 1000 > timeoutSeconds) {
    timedOut = true;
    break;
  }
}
const seconds = (Date.now() - started) / 1000;
let traces = [];
try {
  traces = solver.getOutputSimpleRouteJson()?.traces ?? [];
} catch (error) {
  traces = [];
}

// Session output in DSN units.
// The session is written at the DSN resolution (units per micrometre).
const sesUnitsPerMm = unitsPerMm * Number(resolution[2] ?? "1");
const toUnits = (value) => Math.round(value * sesUnitsPerMm);
const wires = new Map();
let viaCount = 0;
let lengthMm = 0;
for (const trace of traces) {
  const net = trace.connection_name ?? trace.connectionName;
  const lines = wires.get(net) ?? [];
  let run = [];
  let runLayer = null;
  let last = null;
  const flush = () => {
    if (run.length >= 2) lines.push(`        (wire\n          (path ${backMap.get(runLayer)} ${toUnits(traceWidth)}\n${run.map((p) => `            ${toUnits(p.x)} ${toUnits(p.y)}`).join("\n")}\n          )\n        )`);
    run = [];
  };
  for (const point of trace.route) {
    if (point.route_type === "wire") {
      if (runLayer !== point.layer) {
        flush();
        runLayer = point.layer;
      }
      if (last) lengthMm += Math.hypot(point.x - last.x, point.y - last.y);
      run.push(point);
      last = point;
    } else if (point.route_type === "via") {
      flush();
      viaCount += 1;
      lines.push(`        (via "${viaName}" ${toUnits(point.x)} ${toUnits(point.y)})`);
      runLayer = null;
      last = point;
    }
  }
  flush();
  wires.set(net, lines);
}
const netsOut = [...wires].map(([net, lines]) => `      (net "${net}"\n${lines.join("\n")}\n      )`).join("\n");
const session = `(session ${sesPath.split("/").pop()}\n  (base_design ${dsnPath.split("/").pop()})\n  (placement\n    (resolution ${resolution[1]} ${resolution[2]})\n  )\n  (was_is\n  )\n  (routes\n    (resolution ${resolution[1]} ${resolution[2]})\n    (parser\n      (host_cad "pcb-maker tscircuit bridge")\n      (host_version "1")\n    )\n    (library_out\n      (padstack "${viaName}"\n${viaShapes.join("\n")}\n        (attach off)\n      )\n    )\n    (network_out\n${netsOut}\n    )\n  )\n)\n`;
writeFileSync(sesPath, session);
writeFileSync(
  reportPath,
  JSON.stringify(
    {
      solved: solver.solved,
      failed: solver.failed,
      error: solver.error ?? null,
      timed_out: timedOut,
      seconds,
      connections: connections.length,
      traces: traces.length,
      vias: viaCount,
      length_mm: lengthMm,
      obstacles: obstacles.length,
    },
    null,
    2,
  ) + "\n",
);
console.log(JSON.stringify({ solved: solver.solved, failed: solver.failed, seconds, traces: traces.length, vias: viaCount, length_mm: lengthMm }));
