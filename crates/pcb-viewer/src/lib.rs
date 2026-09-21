use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::{
    Problem,
    geometry::{add, rotate_degrees},
    model::CopperShape,
};
use pcb_core::Frame;
use pcb_validate::{CandidateArtifact, ExactValidationAssessment};

/// Render a stable, dependency-free layer image for experiment journals.
/// Coordinates use the same top-view orientation on both files so front/back
/// images can be compared directly while scrolling.
pub fn render_candidate_layer_svg(
    title: &str,
    problem: &Problem,
    candidate: &CandidateArtifact,
    layer: Option<&str>,
) -> String {
    let bounds = problem.board.bounds;
    let width = bounds.width();
    let height = bounds.height();
    let margin = width.max(height) * 0.025 + 0.5;
    let view_x = bounds.min.x - margin;
    let view_y = bounds.min.y - margin;
    let view_width = width + margin * 2.0;
    let view_height = height + margin * 2.0;
    let pixel_width = 1200_u32;
    let pixel_height = ((pixel_width as f64 * view_height / view_width).round() as u32).max(300);
    let copper = if layer == problem.board.layers.first().map(|item| item.id.as_str()) {
        "#4db6ff"
    } else {
        "#ffbd59"
    };
    let poses = candidate
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let mut body = String::new();
    body.push_str(&format!(
        r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#0d1723"/>"##,
        fmt(view_x),
        fmt(view_y),
        fmt(view_width),
        fmt(view_height)
    ));
    body.push_str(&format!(
        r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#101d2b" stroke="#91a8ba" stroke-width="0.18"/>"##,
        fmt(bounds.min.x),
        fmt(bounds.min.y),
        fmt(width),
        fmt(height)
    ));
    for declared in &problem.components {
        let Some(pose) = poses.get(declared.id.as_str()) else {
            continue;
        };
        let fill = if declared.body_is_routing_keepout {
            "#72313d"
        } else {
            "#283e54"
        };
        body.push_str(&format!(
            r##"<g transform="translate({} {}) rotate({})"><rect x="{}" y="{}" width="{}" height="{}" rx="0.18" fill="{}" fill-opacity="0.82" stroke="#93a9bb" stroke-width="0.12"/></g>"##,
            fmt(pose.position.x),
            fmt(pose.position.y),
            fmt(pose.rotation_degrees),
            fmt(-pose.size.x * 0.5),
            fmt(-pose.size.y * 0.5),
            fmt(pose.size.x),
            fmt(pose.size.y),
            fill
        ));
        for keepout in &declared.routing_keepouts {
            if layer != Some(keepout.layer.as_str()) {
                continue;
            }
            let center = add(
                pose.position,
                rotate_degrees(keepout.offset, pose.rotation_degrees),
            );
            match &keepout.shape {
                CopperShape::Circle { diameter } => body.push_str(&format!(
                    r##"<circle cx="{}" cy="{}" r="{}" fill="#d78b42" fill-opacity="0.28" stroke="#ffb45b" stroke-width="0.12" stroke-dasharray="0.28 0.16"/>"##,
                    fmt(center.x),
                    fmt(center.y),
                    fmt(*diameter * 0.5)
                )),
                CopperShape::Rect {
                    size,
                    rotation_degrees,
                } => body.push_str(&format!(
                    r##"<g transform="translate({} {}) rotate({})"><rect x="{}" y="{}" width="{}" height="{}" fill="#d78b42" fill-opacity="0.28" stroke="#ffb45b" stroke-width="0.12" stroke-dasharray="0.28 0.16"/></g>"##,
                    fmt(center.x),
                    fmt(center.y),
                    fmt(pose.rotation_degrees + *rotation_degrees),
                    fmt(-size.x * 0.5),
                    fmt(-size.y * 0.5),
                    fmt(size.x),
                    fmt(size.y)
                )),
            }
        }
        for pin in &declared.pins {
            for pad in &pin.pads {
                if layer != Some(pad.layer.as_str()) {
                    continue;
                }
                let center = add(
                    pose.position,
                    rotate_degrees(pin.pad_local_center(pad), pose.rotation_degrees),
                );
                match &pad.shape {
                    CopperShape::Circle { diameter } => body.push_str(&format!(
                        r##"<circle cx="{}" cy="{}" r="{}" fill="{}" fill-opacity="0.9" stroke="#e7f4ff" stroke-width="0.08"/>"##,
                        fmt(center.x),
                        fmt(center.y),
                        fmt(*diameter * 0.5),
                        copper
                    )),
                    CopperShape::Rect {
                        size,
                        rotation_degrees,
                    } => body.push_str(&format!(
                        r##"<g transform="translate({} {}) rotate({})"><rect x="{}" y="{}" width="{}" height="{}" fill="{}" fill-opacity="0.9" stroke="#e7f4ff" stroke-width="0.08"/></g>"##,
                        fmt(center.x),
                        fmt(center.y),
                        fmt(pose.rotation_degrees + *rotation_degrees),
                        fmt(-size.x * 0.5),
                        fmt(-size.y * 0.5),
                        fmt(size.x),
                        fmt(size.y),
                        copper
                    )),
                }
            }
        }
        body.push_str(&format!(
            r##"<text x="{}" y="{}" fill="#d9e8f3" font-size="0.55" font-family="monospace">{}</text>"##,
            fmt(pose.position.x + pose.size.x * 0.55),
            fmt(pose.position.y - pose.size.y * 0.55),
            escape_xml(&declared.id)
        ));
    }
    if let Some(layer) = layer {
        for trace in &candidate.traces {
            for (index, segment) in trace.points.windows(2).enumerate() {
                if trace.segment_layers.get(index).map(String::as_str) != Some(layer) {
                    continue;
                }
                body.push_str(&format!(
                    r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="{}" stroke-linecap="round"/>"##,
                    fmt(segment[0].x),
                    fmt(segment[0].y),
                    fmt(segment[1].x),
                    fmt(segment[1].y),
                    copper,
                    fmt(trace.width)
                ));
            }
            for via in &trace.vias {
                if via.from_layer != layer && via.to_layer != layer {
                    continue;
                }
                body.push_str(&format!(
                    r##"<circle cx="{}" cy="{}" r="{}" fill="#111821" stroke="#f4e285" stroke-width="0.16"/>"##,
                    fmt(via.position.x),
                    fmt(via.position.y),
                    fmt(via.diameter * 0.5)
                ));
            }
        }
        let routed = candidate
            .traces
            .iter()
            .map(|trace| trace.branch.as_str())
            .collect::<BTreeSet<_>>();
        for net in problem
            .nets
            .iter()
            .filter(|net| net.layer == layer && !routed.contains(net.id.as_str()))
        {
            let Some(from) =
                solved_pin_position(problem, &poses, &net.from.component, &net.from.pin)
            else {
                continue;
            };
            let Some(to) = solved_pin_position(problem, &poses, &net.to.component, &net.to.pin)
            else {
                continue;
            };
            body.push_str(&format!(
                r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="#ff6363" stroke-width="0.16" stroke-dasharray="0.65 0.45"/>"##,
                fmt(from.x),
                fmt(from.y),
                fmt(to.x),
                fmt(to.y)
            ));
        }
    }
    let side = layer.unwrap_or("back (no copper layer)");
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{pixel_width}" height="{pixel_height}" viewBox="{} {} {} {}" role="img" aria-label="{}"><title>{} — {}</title>{}<text x="{}" y="{}" fill="#f3f7fa" font-size="0.7" font-family="monospace">{} — {}</text></svg>"##,
        fmt(view_x),
        fmt(view_y),
        fmt(view_width),
        fmt(view_height),
        escape_xml(title),
        escape_xml(title),
        escape_xml(side),
        body,
        fmt(bounds.min.x + 0.35),
        fmt(bounds.min.y + 0.85),
        escape_xml(title),
        escape_xml(side)
    )
}

fn solved_pin_position(
    problem: &Problem,
    poses: &BTreeMap<&str, &pcb_validate::SolvedComponent>,
    component_id: &str,
    pin_id: &str,
) -> Option<layout_trace_model::Vec2> {
    let declared = problem
        .components
        .iter()
        .find(|component| component.id == component_id)?;
    let pose = poses.get(component_id)?;
    let pin = declared.pins.iter().find(|pin| pin.id == pin_id)?;
    Some(add(
        pose.position,
        rotate_degrees(pin.offset, pose.rotation_degrees),
    ))
}

fn fmt(value: f64) -> String {
    format!("{value:.6}")
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn render_html(title: &str, frames: &[Frame]) -> Result<String, serde_json::Error> {
    let frames = serde_json::to_string(frames)?;
    let title = escape_html(title);
    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title>
<style>
:root {{ color-scheme: dark; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }}
body {{ margin: 0; background: #091019; color: #dce8f2; display: grid; grid-template-rows: auto 1fr; height: 100vh; }}
header {{ display: flex; gap: 14px; align-items: center; padding: 10px 14px; border-bottom: 1px solid #243244; }}
main {{ min-height: 0; display: grid; grid-template-columns: 1fr 340px; }}
canvas {{ width: 100%; height: 100%; background: #0d1723; }}
aside {{ padding: 12px; overflow: auto; border-left: 1px solid #243244; }}
input[type=range] {{ flex: 1; }}
pre {{ white-space: pre-wrap; color: #a9bed0; }}
.hint {{ color: #7590a5; font-size: 12px; }}
</style>
</head>
<body>
<header><strong>{title}</strong><button id="play">Play</button><input id="step" type="range" min="0" value="0"><span id="counter"></span></header>
<main><canvas id="board"></canvas><aside><div class="hint">Field heat, rigid bodies, body-local attachments, particles, constraints, and actual correction vectors are engine evidence. Red links carry residual.</div><pre id="metrics"></pre><pre id="selection">Click a particle.</pre></aside></main>
<script>
const frames={frames};
const canvas=document.querySelector('#board'),ctx=canvas.getContext('2d');
const slider=document.querySelector('#step'),counter=document.querySelector('#counter'),metrics=document.querySelector('#metrics'),selection=document.querySelector('#selection'),play=document.querySelector('#play');
slider.max=Math.max(0,frames.length-1);let selected=null,timer=null,screenParticles=[];
function resize(){{const r=canvas.getBoundingClientRect(),d=devicePixelRatio||1;canvas.width=Math.round(r.width*d);canvas.height=Math.round(r.height*d);ctx.setTransform(d,0,0,d,0,0);draw();}}
function transform(frame){{const r=canvas.getBoundingClientRect(),b=frame.bounds,s=Math.min((r.width-40)/(b.max.x-b.min.x),(r.height-40)/(b.max.y-b.min.y));return {{x:p=>20+(p.x-b.min.x)*s,y:p=>20+(p.y-b.min.y)*s,s}};}}
function rotateRadians(p,a){{const c=Math.cos(a),s=Math.sin(a);return {{x:p.x*c-p.y*s,y:p.x*s+p.y*c}};}}
function bodyPoints(body,t){{return [{{x:-body.half_size.x,y:-body.half_size.y}},{{x:body.half_size.x,y:-body.half_size.y}},{{x:body.half_size.x,y:body.half_size.y}},{{x:-body.half_size.x,y:body.half_size.y}}].map(p=>rotateRadians(p,body.angle_radians)).map(p=>({{x:t.x({{x:p.x+body.position.x}}),y:t.y({{y:p.y+body.position.y}})}}));}}
function draw(){{if(!frames.length)return;const frame=frames[+slider.value],r=canvas.getBoundingClientRect(),t=transform(frame);ctx.clearRect(0,0,r.width,r.height);ctx.strokeStyle='#536579';ctx.strokeRect(t.x(frame.bounds.min),t.y(frame.bounds.min),(frame.bounds.max.x-frame.bounds.min.x)*t.s,(frame.bounds.max.y-frame.bounds.min.y)*t.s);
 if(frame.field){{const f=frame.field,b=frame.bounds,cw=(b.max.x-b.min.x)/(f.width-1),ch=(b.max.y-b.min.y)/(f.height-1),m=f.max_value||1;for(let y=0;y<f.height;y++)for(let x=0;x<f.width;x++){{const v=f.values[y*f.width+x]/m;if(v<.015)continue;ctx.fillStyle=`rgba(245,86,74,${{Math.min(.55,v*.55)}})`;ctx.fillRect(t.x({{x:b.min.x+x*cw}}),t.y({{y:b.min.y+y*ch}}),cw*t.s+1,ch*t.s+1);}}}}
 for(const body of frame.bodies||[]){{const points=bodyPoints(body,t);ctx.beginPath();points.forEach((p,i)=>i?ctx.lineTo(p.x,p.y):ctx.moveTo(p.x,p.y));ctx.closePath();ctx.fillStyle='#31465c99';ctx.fill();ctx.strokeStyle='#c792ea';ctx.lineWidth=2;ctx.stroke();ctx.fillStyle='#d9c2ef';ctx.font='11px ui-monospace';ctx.fillText(body.label,t.x(body.position)+5,t.y(body.position)-5);}}
 const byId=new Map([...frame.particles,...(frame.bodies||[])].map(item=>[item.id,item]));for(const c of frame.constraints){{const a=byId.get(c.first),b=byId.get(c.second);if(!a||!b)continue;ctx.beginPath();ctx.moveTo(t.x(a.position),t.y(a.position));ctx.lineTo(t.x(b.position),t.y(b.position));ctx.strokeStyle=c.residual>1e-3?'#ff6b6b':c.family==='segment_body_clearance'?'#f0a35e':c.family==='maximum_distance'?'#52728e':'#65b889';ctx.lineWidth=c.residual>1e-3?2:1;ctx.stroke();}}
 for(const a of frame.attachments||[]){{const p=byId.get(a.particle);if(!p)continue;ctx.beginPath();ctx.moveTo(t.x(p.position),t.y(p.position));ctx.lineTo(t.x(a.target),t.y(a.target));ctx.strokeStyle=a.residual>1e-3?'#ff6b6b':'#f0a35e';ctx.lineWidth=a.residual>1e-3?2:1;ctx.setLineDash([3,3]);ctx.stroke();ctx.setLineDash([]);}}
 for(const v of frame.vectors){{ctx.beginPath();ctx.moveTo(t.x(v.origin),t.y(v.origin));ctx.lineTo(t.x({{x:v.origin.x+v.vector.x*8}}),t.y({{y:v.origin.y+v.vector.y*8}}));ctx.strokeStyle='#ffd166';ctx.lineWidth=1.5;ctx.stroke();}}
 screenParticles=[];for(const p of frame.particles){{const x=t.x(p.position),y=t.y(p.position),color=p.role==='trace'?'#49a7ff':p.role==='terminal'?'#ffd166':'#c792ea';ctx.beginPath();ctx.arc(x,y,p.id===selected?6:4,0,Math.PI*2);ctx.fillStyle=color;ctx.fill();if(p.inverse_mass===0){{ctx.strokeStyle='#fff';ctx.stroke();}}screenParticles.push({{p,x,y}});}}
 const worst=[...frame.constraints,...(frame.attachments||[]).map(a=>({{...a,family:'body_attachment'}}))].sort((a,b)=>b.residual-a.residual).slice(0,5).map(c=>({{label:c.label,family:c.family,residual:c.residual}}));
 counter.textContent=`${{+slider.value+1}} / ${{frames.length}}`;metrics.textContent=JSON.stringify({{step:frame.step,...frame.metrics,particles:frame.particles.length,bodies:(frame.bodies||[]).length,constraints:frame.constraints.length,attachments:(frame.attachments||[]).length,field_vectors:frame.vectors.length,worst_constraints:worst}},null,2);}}
slider.oninput=draw;canvas.onclick=e=>{{const r=canvas.getBoundingClientRect(),x=e.clientX-r.left,y=e.clientY-r.top,hit=screenParticles.map(q=>({{...q,d:Math.hypot(q.x-x,q.y-y)}})).sort((a,b)=>a.d-b.d)[0];if(hit&&hit.d<12){{selected=hit.p.id;selection.textContent=JSON.stringify(hit.p,null,2);draw();}}}};
play.onclick=()=>{{if(timer){{clearInterval(timer);timer=null;play.textContent='Play';return;}}play.textContent='Pause';timer=setInterval(()=>{{slider.value=(+slider.value+1)%frames.length;draw();}},120);}};
addEventListener('resize',resize);resize();
</script>
</body>
</html>"#
    ))
}

/// Render one durable route candidate and its exact-gate/search evidence.
/// The viewer consumes semantic artifacts only; it never reconstructs solver
/// correctness from pixels.
pub fn render_candidate_html(
    title: &str,
    problem: &layout_trace_model::Problem,
    candidate: &CandidateArtifact,
    validation: &ExactValidationAssessment,
    evidence: &serde_json::Value,
) -> Result<String, serde_json::Error> {
    let payload = serde_json::to_string(&serde_json::json!({
        "problem": problem,
        "candidate": candidate,
        "validation": validation,
        "evidence": evidence,
    }))?
    .replace("</", "<\\/");
    let title = escape_html(title);
    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title>
<style>
:root {{ color-scheme: dark; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }}
* {{ box-sizing: border-box; }}
body {{ margin: 0; background: #091019; color: #dce8f2; display: grid; grid-template-rows: auto 1fr; height: 100vh; }}
header {{ display: flex; gap: 14px; align-items: center; padding: 10px 14px; border-bottom: 1px solid #243244; }}
header .spacer {{ flex: 1; }}
header input[type=range] {{ width: 220px; }}
.pass {{ color: #65d68a; }} .fail {{ color: #ff7474; }}
main {{ min-height: 0; display: grid; grid-template-columns: minmax(0,1fr) 390px; }}
canvas {{ width: 100%; height: 100%; background: #0d1723; }}
aside {{ padding: 12px; overflow: auto; border-left: 1px solid #243244; }}
select {{ color: inherit; background: #132334; border: 1px solid #36506a; padding: 4px; }}
h3 {{ font-size: 13px; margin: 14px 0 5px; color: #88a9c3; }}
pre {{ white-space: pre-wrap; overflow-wrap: anywhere; margin: 0; color: #b9cddd; font-size: 12px; }}
.hint {{ color: #7590a5; font-size: 11px; }}
</style>
</head>
<body>
<header><strong>{title}</strong><span id="status"></span><button id="play">Play</button><input id="attempt" type="range" min="0" value="0"><span id="counter"></span><span class="spacer"></span><label>Layer <select id="layer"></select></label></header>
<main><canvas id="board"></canvas><aside><div class="hint">Use playback to inspect retained routing and topology attempts. Gold arrows are proposed component motion; cyan circles and lines show classified blocked-frontier centroids; magenta diamonds are explicit junction nodes; green rings show committed shared-tree attachments. Small blue/gray rings show selected/rejected attachment-portfolio sources. An amber ring marks a searched source moved by prefix trimming; a violet halo marks a selected terminal-junction alternative; a white X marks a contact inserted inside one or both segments. Solid copper and exact evidence remain authoritative.</div><h3>Attempt</h3><pre id="attempt-info"></pre><h3>Routing work</h3><pre id="work"></pre><h3>Failed branches</h3><pre id="failures"></pre><h3>Exact findings</h3><pre id="violations"></pre></aside></main>
<script>
const data={payload};
const canvas=document.querySelector('#board'),ctx=canvas.getContext('2d'),layerSelect=document.querySelector('#layer'),attemptSlider=document.querySelector('#attempt'),play=document.querySelector('#play'),counter=document.querySelector('#counter');
const layers=data.problem.board.layers.map(l=>l.id);for(const id of ['all',...layers]){{const o=document.createElement('option');o.value=id;o.textContent=id;layerSelect.append(o);}}
const colors=['#4db6ff','#ffbd59','#c792ea','#5dd39e','#ff6b8a','#9fa8ff'];
const layerColor=id=>colors[Math.max(0,layers.indexOf(id))%colors.length];
const declared=new Map(data.problem.components.map(c=>[c.id,c]));let poses=new Map();
const attempts=data.evidence.attempts||[];const retained=attempts.map((attempt,index)=>{{const direct=attempt.routing||(attempt.candidate?{{candidate:attempt.candidate,validation:attempt.validation,evidence:data.evidence.routing||data.evidence}}:null),parent=attempts[attempt.parent_attempt??0],base=direct||parent?.routing||(parent?.candidate?{{candidate:parent.candidate,validation:parent.validation,evidence:data.evidence.routing||data.evidence}}:null);return {{attempt,index,base,projected:!direct}};}}).filter(frame=>frame.base?.candidate);
const frames=retained.length?retained:[{{attempt:{{id:'selected'}},base:{{candidate:data.candidate,validation:data.validation,evidence:data.evidence.routing||data.evidence}},index:0,projected:false}}];
attemptSlider.max=Math.max(0,frames.length-1);let selectedFrame=frames.findIndex(frame=>frame.index===data.evidence.selected_attempt);attemptSlider.value=selectedFrame>=0?selectedFrame:frames.length-1;play.disabled=frames.length<2;let timer=null;
const rotate=(p,a)=>{{const r=a*Math.PI/180,c=Math.cos(r),s=Math.sin(r);return {{x:p.x*c-p.y*s,y:p.x*s+p.y*c}};}};
const add=(a,b)=>({{x:a.x+b.x,y:a.y+b.y}});
function padCenter(component,pin,pad){{const pose=poses.get(component.id),local=pad.local_center||pin.offset||{{x:0,y:0}};return add(pose.position,rotate(local,pose.rotation_degrees));}}
function terminal(ref){{const component=declared.get(ref.component),pin=component?.pins.find(p=>p.id===ref.pin),pad=pin?.pads?.[0];if(!component||!pin)return null;return padCenter(component,pin,pad||{{}});}}
function resize(){{const r=canvas.getBoundingClientRect(),d=devicePixelRatio||1;canvas.width=Math.round(r.width*d);canvas.height=Math.round(r.height*d);ctx.setTransform(d,0,0,d,0,0);draw();}}
function transform(b){{const r=canvas.getBoundingClientRect(),s=Math.min((r.width-50)/(b.max.x-b.min.x),(r.height-50)/(b.max.y-b.min.y));return {{x:p=>25+(p.x-b.min.x)*s,y:p=>25+(p.y-b.min.y)*s,s}};}}
function arrow(origin,vector,color,t){{const magnitude=Math.hypot(vector.x,vector.y);if(magnitude<1e-12)return;const scale=Math.min(3,Math.max(.8,magnitude)),tip={{x:origin.x+vector.x/magnitude*scale,y:origin.y+vector.y/magnitude*scale}},angle=Math.atan2(tip.y-origin.y,tip.x-origin.x),head=.28;ctx.beginPath();ctx.moveTo(t.x(origin),t.y(origin));ctx.lineTo(t.x(tip),t.y(tip));ctx.lineTo(t.x({{x:tip.x-head*Math.cos(angle-.55),y:tip.y-head*Math.sin(angle-.55)}}),t.y({{x:tip.x-head*Math.cos(angle-.55),y:tip.y-head*Math.sin(angle-.55)}}));ctx.moveTo(t.x(tip),t.y(tip));ctx.lineTo(t.x({{x:tip.x-head*Math.cos(angle+.55),y:tip.y-head*Math.sin(angle+.55)}}),t.y({{x:tip.x-head*Math.cos(angle+.55),y:tip.y-head*Math.sin(angle+.55)}}));ctx.strokeStyle=color;ctx.lineWidth=2;ctx.stroke();}}
function polygon(points,fill,stroke,width=1,dash=[]){{ctx.beginPath();points.forEach((p,i)=>i?ctx.lineTo(p.x,p.y):ctx.moveTo(p.x,p.y));ctx.closePath();ctx.fillStyle=fill;ctx.fill();ctx.strokeStyle=stroke;ctx.lineWidth=width;ctx.setLineDash(dash);ctx.stroke();ctx.setLineDash([]);}}
function rectPoints(center,size,angle,t){{return [{{x:-size.x/2,y:-size.y/2}},{{x:size.x/2,y:-size.y/2}},{{x:size.x/2,y:size.y/2}},{{x:-size.x/2,y:size.y/2}}].map(p=>add(center,rotate(p,angle))).map(p=>({{x:t.x(p),y:t.y(p)}}));}}
function drawPad(component,pin,pad,t,selectedLayer){{if(selectedLayer!=='all'&&pad.layer!==selectedLayer)return;const center=padCenter(component,pin,pad),pose=poses.get(component.id),color=layerColor(pad.layer);if(pad.shape.kind==='circle'){{ctx.beginPath();ctx.arc(t.x(center),t.y(center),pad.shape.diameter*t.s/2,0,Math.PI*2);ctx.fillStyle=color+'99';ctx.fill();ctx.strokeStyle=color;ctx.stroke();}}else{{polygon(rectPoints(center,pad.shape.size,pose.rotation_degrees+(pad.shape.rotation_degrees||0),t),color+'77',color);}}}}
function drawKeepout(component,keepout,t,selectedLayer){{if(selectedLayer!=='all'&&keepout.layer!==selectedLayer)return;const pose=poses.get(component.id),center=add(pose.position,rotate(keepout.offset||{{x:0,y:0}},pose.rotation_degrees));ctx.setLineDash([5,3]);if(keepout.shape.kind==='circle'){{ctx.beginPath();ctx.arc(t.x(center),t.y(center),keepout.shape.diameter*t.s/2,0,Math.PI*2);ctx.fillStyle='#d78b4244';ctx.fill();ctx.strokeStyle='#ffb45b';ctx.stroke();}}else{{polygon(rectPoints(center,keepout.shape.size,pose.rotation_degrees+(keepout.shape.rotation_degrees||0),t),'#d78b4244','#ffb45b',1,[5,3]);}}ctx.setLineDash([]);}}
function draw(){{const frame=frames[+attemptSlider.value],attempt=frame.attempt,routing=frame.base,candidate=routing.candidate,rejection=attempt.rejected_before_routing||attempt.rejected_before_validation,validation=attempt.routing?.validation||attempt.validation||(rejection?{{complete:false,violations:[{{message:rejection}}]}}:routing.validation||data.validation),routingEvidence=routing.evidence||data.evidence.routing||data.evidence;poses=new Map(candidate.components.map(c=>[c.id,{{...c,position:{{...c.position}}}}]));const moved=attempt.moved_components?.length?attempt.moved_components:(attempt.blocker?[attempt.blocker]:[]);if(frame.projected&&attempt.displacement)for(const id of moved){{const pose=poses.get(id);if(pose)pose.position={{x:pose.position.x+attempt.displacement.x,y:pose.position.y+attempt.displacement.y}};}}const r=canvas.getBoundingClientRect(),b=attempt.board_bounds||data.problem.board.bounds,t=transform(b),selectedLayer=layerSelect.value||'all';ctx.clearRect(0,0,r.width,r.height);ctx.strokeStyle='#6d8294';ctx.lineWidth=2;ctx.strokeRect(t.x(b.min),t.y(b.min),(b.max.x-b.min.x)*t.s,(b.max.y-b.min.y)*t.s);
 for(const component of data.problem.components){{const pose=poses.get(component.id);if(!pose)continue;polygon(rectPoints(pose.position,component.size,pose.rotation_degrees,t),component.body_is_routing_keepout?'#8e3b4688':'#24364b99',component.body_is_routing_keepout?'#e56b75':'#7892a8');for(const keepout of component.routing_keepouts||[])drawKeepout(component,keepout,t,selectedLayer);ctx.fillStyle='#d7e4ef';ctx.font='11px ui-monospace';ctx.fillText(component.id,t.x(pose.position)+4,t.y(pose.position)-4);for(const pin of component.pins||[])for(const pad of pin.pads||[])drawPad(component,pin,pad,t,selectedLayer);}}
 for(const trace of candidate.traces){{for(let i=0;i<trace.points.length-1;i++){{const layer=trace.segment_layers[i];if(selectedLayer!=='all'&&layer!==selectedLayer)continue;ctx.beginPath();ctx.moveTo(t.x(trace.points[i]),t.y(trace.points[i]));ctx.lineTo(t.x(trace.points[i+1]),t.y(trace.points[i+1]));ctx.strokeStyle=layerColor(layer);ctx.lineWidth=Math.max(2,trace.width*t.s);ctx.lineCap='round';ctx.stroke();}}for(const via of trace.vias){{if(selectedLayer!=='all'&&selectedLayer!==via.from_layer&&selectedLayer!==via.to_layer)continue;ctx.beginPath();ctx.arc(t.x(via.position),t.y(via.position),Math.max(3,via.diameter*t.s/2),0,Math.PI*2);ctx.fillStyle='#101820';ctx.fill();ctx.strokeStyle='#f4e285';ctx.lineWidth=2;ctx.stroke();}}}}
 for(const graph of candidate.route_graphs||[])for(const node of graph.nodes||[]){{if(node.kind!=='junction'||(selectedLayer!=='all'&&selectedLayer!==node.layer))continue;const x=t.x(node.position),y=t.y(node.position),r=6;ctx.beginPath();ctx.moveTo(x,y-r);ctx.lineTo(x+r,y);ctx.lineTo(x,y+r);ctx.lineTo(x-r,y);ctx.closePath();ctx.fillStyle='#f472d0';ctx.fill();ctx.strokeStyle='#ffd0f2';ctx.lineWidth=1.5;ctx.stroke();ctx.fillStyle='#ffd0f2';ctx.font='10px ui-monospace';ctx.fillText(node.id,x+8,y-6);}}
 for(const branch of routingEvidence.branches||[]){{const trials=branch.tree_attachment_attempts||[];if(trials.length<2)continue;for(const trial of trials){{if(selectedLayer!=='all'&&selectedLayer!==trial.layer)continue;const x=t.x(trial.position),y=t.y(trial.position);if(trial.committed_position&&(trial.committed_position.x!==trial.position.x||trial.committed_position.y!==trial.position.y)){{ctx.beginPath();ctx.moveTo(x,y);ctx.lineTo(t.x(trial.committed_position),t.y(trial.committed_position));ctx.strokeStyle=trial.selected?'#38bdf866':'#64748b55';ctx.lineWidth=1;ctx.setLineDash([2,3]);ctx.stroke();ctx.setLineDash([]);}}ctx.beginPath();ctx.arc(x,y,trial.selected?6:4,0,Math.PI*2);ctx.strokeStyle=trial.selected?'#38bdf8':'#64748b';ctx.lineWidth=trial.selected?2:1;ctx.stroke();}}}}
 for(const branch of routingEvidence.branches||[]){{const attachment=branch.tree_attachment;if(!attachment||(selectedLayer!=='all'&&selectedLayer!==attachment.layer))continue;const x=t.x(attachment.position),y=t.y(attachment.position);if(attachment.trimmed_prefix_points>0&&attachment.searched_position&&(selectedLayer==='all'||selectedLayer===attachment.searched_layer)){{const sx=t.x(attachment.searched_position),sy=t.y(attachment.searched_position);ctx.beginPath();ctx.moveTo(sx,sy);ctx.lineTo(x,y);ctx.strokeStyle='#fbbf2466';ctx.lineWidth=1.5;ctx.setLineDash([4,4]);ctx.stroke();ctx.setLineDash([]);ctx.beginPath();ctx.arc(sx,sy,8,0,Math.PI*2);ctx.strokeStyle='#fbbf24';ctx.lineWidth=2;ctx.stroke();}}if(attachment.terminal_preference_selected){{ctx.beginPath();ctx.arc(x,y,14,0,Math.PI*2);ctx.strokeStyle='#a78bfa';ctx.lineWidth=2;ctx.stroke();}}if(attachment.existing_segment_split||attachment.new_segment_split){{ctx.beginPath();ctx.moveTo(x-6,y-6);ctx.lineTo(x+6,y+6);ctx.moveTo(x+6,y-6);ctx.lineTo(x-6,y+6);ctx.strokeStyle='#f8fafc';ctx.lineWidth=2;ctx.stroke();}}ctx.beginPath();ctx.arc(x,y,10,0,Math.PI*2);ctx.strokeStyle='#4ade80';ctx.lineWidth=2;ctx.setLineDash([3,3]);ctx.stroke();ctx.setLineDash([]);ctx.fillStyle='#bbf7d0';ctx.font='10px ui-monospace';ctx.fillText(`${{branch.branch}} → ${{attachment.target_terminal}}`,x+12,y+12);}}
 const routed=new Set(candidate.traces.map(t=>t.branch));for(const net of data.problem.nets||[]){{if(routed.has(net.id))continue;const a=terminal(net.from),b=terminal(net.to);if(!a||!b)continue;ctx.beginPath();ctx.moveTo(t.x(a),t.y(a));ctx.lineTo(t.x(b),t.y(b));ctx.strokeStyle='#ff6363';ctx.lineWidth=1.5;ctx.setLineDash([7,5]);ctx.stroke();ctx.setLineDash([]);}}
 for(const branch of routingEvidence.branches||[])for(const blocker of branch.blockers||[]){{const p=blocker.frontier_centroid;if(!p)continue;ctx.beginPath();ctx.arc(t.x(p),t.y(p),4,0,Math.PI*2);ctx.fillStyle='#67e8f9';ctx.fill();const owner=poses.get(blocker.object);if(owner){{ctx.beginPath();ctx.moveTo(t.x(p),t.y(p));ctx.lineTo(t.x(owner.position),t.y(owner.position));ctx.strokeStyle='#67e8f988';ctx.lineWidth=1.5;ctx.stroke();}}}}
 if(attempt.displacement)for(const id of moved){{const owner=poses.get(id),move=attempt.displacement;if(owner)arrow({{x:owner.position.x-move.x,y:owner.position.y-move.y}},move,'#ffd166',t);}}
const complete=validation.complete&&(routingEvidence.failed_branches||0)===0,status=document.querySelector('#status');status.className=complete?'pass':'fail';status.textContent=complete?'exact pass':'incomplete / exact fail';counter.textContent=`${{+attemptSlider.value+1}} / ${{frames.length}}`;document.querySelector('#attempt-info').textContent=JSON.stringify({{attempt_index:frame.index,id:attempt.id,state_id:attempt.state_id,parent_state_id:attempt.parent_state_id,depth:attempt.depth,scale:attempt.scale,board_bounds:attempt.board_bounds,accepted:attempt.accepted,reused_previous_candidate:attempt.reused_previous_candidate,deformation_exact:attempt.deformation_validation?.complete,deformation_findings:attempt.deformation_validation?.violations?.length,motion_processor:attempt.motion_processor,continuous_motion_status:attempt.continuous_motion?.status,continuous_motion_detail:attempt.continuous_motion?.detail,continuous_motion_selected_branches:attempt.continuous_motion?.selected_branches,continuous_motion_selected_bodies:attempt.continuous_motion?.selected_bodies,continuous_motion_selected_obstacles:attempt.continuous_motion?.selected_obstacles,continuous_motion_unsupported_findings:attempt.continuous_motion?.unsupported_findings,continuous_motion_segment_pairs:attempt.continuous_motion?.segment_pairs,continuous_motion_segment_body_pairs:attempt.continuous_motion?.segment_body_pairs,continuous_motion_body_motion:attempt.continuous_motion?.body_motion,continuous_motion_trace_motion:attempt.continuous_motion?.trace_motion,continuous_post_process:attempt.continuous_post_process,local_repair_rounds:attempt.local_repair_rounds,rerouted_branches:attempt.rerouted_branches,retained_branch_count:attempt.retained_branch_count,deformation_removed_points:attempt.deformation_removed_points,action:attempt.action,action_history:attempt.action_history,post_action:attempt.post_action,fingerprint:attempt.fingerprint,score:attempt.score,expansion_error:attempt.expansion_error,iteration:attempt.iteration,parent_attempt:attempt.parent_attempt,proposal:attempt.proposal,trace_length_mm:attempt.trace_length_mm,blocker:attempt.blocker,pressure:attempt.pressure,direction:attempt.direction,displacement:attempt.displacement,moved_components:moved,chain_contacts:attempt.chain_contacts||[],rejected_before_routing:attempt.rejected_before_routing,rejected_before_validation:attempt.rejected_before_validation}},null,2);document.querySelector('#work').textContent=JSON.stringify(routingEvidence,null,2);document.querySelector('#failures').textContent=JSON.stringify((routingEvidence.branches||[]).filter(b=>b.status!=='found'),null,2);document.querySelector('#violations').textContent=JSON.stringify(validation.violations||[],null,2);
}}
attemptSlider.oninput=draw;layerSelect.onchange=draw;play.onclick=()=>{{if(timer){{clearInterval(timer);timer=null;play.textContent='Play';return;}}play.textContent='Pause';timer=setInterval(()=>{{attemptSlider.value=(+attemptSlider.value+1)%frames.length;draw();}},600);}};addEventListener('resize',resize);resize();
</script>
</body>
</html>"#
    ))
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_viewer_is_self_contained() {
        let html = render_html("particle <field>", &[]).unwrap();
        assert!(html.contains("particle &lt;field&gt;"));
        assert!(html.contains("const frames=[]"));
        assert!(!html.contains("https://"));
        assert!(html.contains("body-local attachments"));
        assert!(html.contains("bodyPoints(body,t)"));
        assert!(html.contains("frame.attachments||[]"));
        assert!(html.contains("segment_body_clearance"));
        assert!(html.contains("...frame.particles,...(frame.bodies||[])"));
    }

    #[test]
    fn route_viewer_embeds_candidate_and_failure_evidence() {
        let problem: layout_trace_model::Problem = serde_json::from_str(
            r#"{
                "schema_version":1,
                "board":{"bounds":{"min":{"x":0.0,"y":0.0},"max":{"x":10.0,"y":8.0}},"layers":[{"id":"top"}]},
                "rules":{"clearance":0.2},
                "components":[{"id":"U1","position":{"x":5.0,"y":4.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"fixed","rotation":"fixed"},"body_is_routing_keepout":false,"routing_keepouts":[{"layer":"top","offset":{"x":0.0,"y":0.0},"shape":{"kind":"circle","diameter":3.0}}],"pins":[]}]
            }"#,
        )
        .unwrap();
        let candidate = CandidateArtifact {
            schema_version: pcb_validate::CANDIDATE_SCHEMA_VERSION,
            components: vec![pcb_validate::SolvedComponent {
                id: "U1".into(),
                position: layout_trace_model::Vec2::new(5.0, 4.0),
                size: layout_trace_model::Vec2::new(1.0, 1.0),
                rotation_degrees: 0.0,
            }],
            traces: Vec::new(),
            route_graphs: Vec::new(),
        };
        let validation = pcb_validate::validate_candidate(&problem, &candidate).unwrap();
        let html = render_candidate_html(
            "route <failure>",
            &problem,
            &candidate,
            &validation,
            &serde_json::json!({"failed_branches": 1}),
        )
        .unwrap();
        assert!(html.contains("route &lt;failure&gt;"));
        assert!(html.contains("failed_branches"));
        assert!(html.contains("Use playback to inspect retained routing and topology attempts"));
        assert!(html.contains("id=\"play\""));
        assert!(html.contains("id=\"attempt\""));
        assert!(html.contains("trimmed_prefix_points"));
        assert!(html.contains("#fbbf24"));
        assert!(html.contains("terminal_preference_selected"));
        assert!(html.contains("#a78bfa"));
        assert!(html.contains("state_id:attempt.state_id"));
        assert!(html.contains("action:attempt.action"));
        assert!(html.contains("score:attempt.score"));
        assert!(html.contains("post_action:attempt.post_action"));
        assert!(html.contains("drawKeepout(component,keepout"));
        assert!(!html.contains("https://"));

        let svg = render_candidate_layer_svg("front <route>", &problem, &candidate, Some("top"));
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("front &lt;route&gt;"));
        assert!(svg.contains("U1"));
        assert!(svg.contains("top"));
        assert!(svg.contains("stroke=\"#ffb45b\""));
        assert!(svg.contains("r=\"1.500000\""));
    }

    #[test]
    fn route_viewer_retains_attempt_playback_and_pressure_overlays() {
        let problem: layout_trace_model::Problem = serde_json::from_str(
            r#"{
                "schema_version":1,
                "board":{"bounds":{"min":{"x":0.0,"y":0.0},"max":{"x":10.0,"y":8.0}},"layers":[{"id":"top"}]},
                "rules":{"clearance":0.2},
                "components":[{"id":"WALL","position":{"x":5.0,"y":4.0},"size":{"x":1.0,"y":1.0},"constraints":{"movement":"vertical","rotation":"fixed"},"pins":[]}]
            }"#,
        )
        .unwrap();
        let candidate = CandidateArtifact {
            schema_version: pcb_validate::CANDIDATE_SCHEMA_VERSION,
            components: vec![pcb_validate::SolvedComponent {
                id: "WALL".into(),
                position: layout_trace_model::Vec2::new(5.0, 4.0),
                size: layout_trace_model::Vec2::new(1.0, 1.0),
                rotation_degrees: 0.0,
            }],
            traces: Vec::new(),
            route_graphs: Vec::new(),
        };
        let validation = pcb_validate::validate_candidate(&problem, &candidate).unwrap();
        let evidence = serde_json::json!({
            "selected_attempt": 0,
            "attempts": [{
                "id": "pressure-0001",
                "blocker": "WALL",
                "displacement": {"x": 0.0, "y": 0.5},
                "moved_components": ["WALL"],
                "routing": {
                    "candidate": candidate,
                    "validation": validation,
                    "evidence": {
                        "failed_branches": 1,
                        "branches": [{
                            "status": "no_path",
                            "blockers": [{
                                "object": "WALL",
                                "frontier_centroid": {"x": 4.0, "y": 4.0}
                            }]
                        }]
                    }
                }
            }]
        });
        let html = render_candidate_html(
            "pressure playback",
            &problem,
            &candidate,
            &validation,
            &evidence,
        )
        .unwrap();
        assert!(html.contains("pressure-0001"));
        assert!(html.contains("frontier_centroid"));
        assert!(html.contains("attemptSlider.oninput=draw"));
        assert!(html.contains("play.onclick"));
        assert!(html.contains("magenta diamonds"));
        assert!(html.contains("tree_attachment_attempts"));
        assert!(html.contains("attachment-portfolio sources"));
        assert!(html.contains("node.kind!=='junction'"));
        assert!(html.contains("attempt.candidate"));
    }
}
