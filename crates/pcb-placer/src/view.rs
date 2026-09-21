// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! A self-contained HTML playback of a placement run.

use crate::global::Frame;
use crate::problem::Problem;

pub fn playback_html(problem: &Problem, frames: &[Frame], title: &str) -> String {
    let bounds = problem.bounds();
    let mut data = String::from("{");
    data.push_str(&format!(
        "\"bounds\":[{},{},{},{}],\"outline\":[{}],",
        bounds[0],
        bounds[1],
        bounds[2],
        bounds[3],
        problem
            .outline
            .iter()
            .map(|point| format!("[{:.3},{:.3}]", point[0], point[1]))
            .collect::<Vec<_>>()
            .join(",")
    ));
    data.push_str("\"components\":[");
    for (index, component) in problem.components.iter().enumerate() {
        if index > 0 {
            data.push(',');
        }
        data.push_str(&format!(
            "{{\"name\":{:?},\"round\":{},\"fixed\":{},\"center\":[{:.3},{:.3}],\"size\":[{:.3},{:.3}],\"pins\":[{}]}}",
            component.name,
            component.round,
            component.fixed,
            component.body_center[0],
            component.body_center[1],
            component.body_size[0],
            component.body_size[1],
            component
                .pins
                .iter()
                .map(|pin| format!("[{:.3},{:.3},{}]", pin.offset[0], pin.offset[1], pin.net))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    data.push_str("],\"netWeights\":[");
    data.push_str(
        &problem
            .net_weights
            .iter()
            .map(|weight| format!("{weight:.3}"))
            .collect::<Vec<_>>()
            .join(","),
    );
    data.push_str("],\"frames\":[");
    for (index, frame) in frames.iter().enumerate() {
        if index > 0 {
            data.push(',');
        }
        data.push_str(&format!(
            "{{\"i\":{},\"o\":{:.4},\"w\":{:.2},\"p\":[{}]}}",
            frame.iteration,
            frame.overflow,
            frame.wirelength,
            frame
                .poses
                .iter()
                .map(|pose| format!(
                    "[{:.3},{:.3},{:.1}]",
                    pose.position[0], pose.position[1], pose.angle
                ))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    data.push_str("]}");
    TEMPLATE.replace("__TITLE__", title).replace("__DATA__", &data)
}

const TEMPLATE: &str = r##"<!doctype html>
<html><head><meta charset="utf-8"><title>__TITLE__</title>
<style>
body{margin:0;background:#10151c;color:#d5dde8;font:13px system-ui,sans-serif}
header{display:flex;gap:12px;align-items:center;padding:8px 12px;background:#0b0f14}
input[type=range]{flex:1}
canvas{display:block;margin:8px auto;background:#0d1a14}
button{background:#23303f;color:inherit;border:0;padding:4px 12px;border-radius:4px}
</style></head><body>
<header><b>__TITLE__</b><button id="play">Play</button>
<input id="slider" type="range" min="0" value="0"><span id="info"></span></header>
<canvas id="canvas"></canvas>
<script>
const D = __DATA__;
const canvas = document.getElementById('canvas'), ctx = canvas.getContext('2d');
const slider = document.getElementById('slider'), info = document.getElementById('info');
slider.max = D.frames.length - 1;
const [x0, y0, x1, y1] = D.bounds, pad = 20;
const scale = Math.min((innerWidth - 2 * pad) / (x1 - x0), (innerHeight - 70 - 2 * pad) / (y1 - y0));
canvas.width = (x1 - x0) * scale + 2 * pad; canvas.height = (y1 - y0) * scale + 2 * pad;
const X = x => (x - x0) * scale + pad, Y = y => (y - y0) * scale + pad;
function rot(p, a) { const r = -a * Math.PI / 180, c = Math.cos(r), s = Math.sin(r); return [p[0] * c - p[1] * s, p[0] * s + p[1] * c]; }
function draw(k) {
  const f = D.frames[k];
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.strokeStyle = '#8aa'; ctx.lineWidth = 1.5; ctx.beginPath();
  D.outline.forEach((p, i) => i ? ctx.lineTo(X(p[0]), Y(p[1])) : ctx.moveTo(X(p[0]), Y(p[1])));
  ctx.closePath(); ctx.stroke();
  const nets = {};
  D.components.forEach((c, i) => {
    const [px, py, a] = f.p[i];
    c.pins.forEach(pin => { const o = rot(pin, a); (nets[pin[2]] ||= []).push([px + o[0], py + o[1]]); });
  });
  ctx.lineWidth = 0.6;
  for (const n in nets) {
    const pins = nets[n]; if (pins.length < 2 || pins.length > 12) continue;
    ctx.strokeStyle = 'rgba(255,200,80,0.35)';
    const cx = pins.reduce((s, p) => s + p[0], 0) / pins.length, cy = pins.reduce((s, p) => s + p[1], 0) / pins.length;
    pins.forEach(p => { ctx.beginPath(); ctx.moveTo(X(cx), Y(cy)); ctx.lineTo(X(p[0]), Y(p[1])); ctx.stroke(); });
  }
  D.components.forEach((c, i) => {
    const [px, py, a] = f.p[i];
    const corners = [[-1, -1], [1, -1], [1, 1], [-1, 1]].map(s => {
      const o = rot([c.center[0] + s[0] * c.size[0] / 2, c.center[1] + s[1] * c.size[1] / 2], a);
      return [X(px + o[0]), Y(py + o[1])];
    });
    ctx.beginPath();
    if (c.round) { const m = rot(c.center, a); ctx.arc(X(px + m[0]), Y(py + m[1]), c.size[0] / 2 * scale, 0, 2 * Math.PI); }
    else { corners.forEach((p, j) => j ? ctx.lineTo(p[0], p[1]) : ctx.moveTo(p[0], p[1])); ctx.closePath(); }
    ctx.fillStyle = c.fixed ? 'rgba(150,150,170,0.35)' : 'rgba(80,160,255,0.30)';
    ctx.strokeStyle = c.fixed ? '#99a' : '#5af'; ctx.lineWidth = 1; ctx.fill(); ctx.stroke();
    ctx.fillStyle = '#e8c070';
    c.pins.forEach(pin => { const o = rot(pin, a); ctx.fillRect(X(px + o[0]) - 1, Y(py + o[1]) - 1, 2, 2); });
    if (c.size[0] * scale > 22) { ctx.fillStyle = '#cfe'; ctx.font = '10px sans-serif';
      const m = rot(c.center, a); ctx.fillText(c.name, X(px + m[0]) - 8, Y(py + m[1]) + 3); }
  });
  info.textContent = `iteration ${f.i}  overflow ${f.o.toFixed(3)}  wirelength ${f.w.toFixed(1)} mm`;
}
slider.oninput = () => draw(+slider.value);
let timer = null;
document.getElementById('play').onclick = () => {
  if (timer) { clearInterval(timer); timer = null; return; }
  if (+slider.value >= D.frames.length - 1) slider.value = 0;
  timer = setInterval(() => { if (+slider.value >= D.frames.length - 1) { clearInterval(timer); timer = null; return; }
    slider.value = +slider.value + 1; draw(+slider.value); }, 40);
};
if (location.hash === '#last') slider.value = D.frames.length - 1;
else if (location.hash.startsWith('#')) slider.value = Math.min(D.frames.length - 1, +location.hash.slice(1) || 0);
draw(+slider.value);
</script></body></html>
"##;
