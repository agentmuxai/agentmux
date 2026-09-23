// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Reproduce "hold a key" typing into a focused composer via CDP Input events
// while recording Long Animation Frames in-page, then print an attribution
// summary. Removes the need for a human to type during a capture.
//
//   node scripts/ui-screenshots/synthetic-typing-loaf.mjs <port> <targetIdSubstr> <seconds> [keysPerSec=20] [char=f]
import WebSocket from "ws";

const [port = "9223", targetSel = "", secs = "10", kps = "20", ch = "f"] = process.argv.slice(2);
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page" && (t.id.includes(targetSel) || t.url.includes(targetSel)));
if (!page) { console.error("no page target matching", targetSel); process.exit(1); }

const ws = new WebSocket(page.webSocketDebuggerUrl, { perMessageDeflate: false, maxPayload: 1 << 28 });
await new Promise((r, e) => { ws.once("open", r); ws.once("error", e); });
let id = 0; const pending = new Map();
ws.on("message", (m) => { const j = JSON.parse(m); if (j.id && pending.has(j.id)) { pending.get(j.id)(j); pending.delete(j.id); } });
const send = (method, params = {}) => new Promise((res, rej) => { const i = ++id; pending.set(i, (j) => (j.error ? rej(new Error(`${method}: ${j.error.message}`)) : res(j))); ws.send(JSON.stringify({ id: i, method, params })); });
const evalIn = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails.exception?.description || r.result.exceptionDetails));
  return r.result?.result?.value;
};

// 1. Which textarea is focused? Refuse to type into nothing.
const focus = await evalIn(`(()=>{const a=document.activeElement;return {tag:a&&a.tagName,composer:!!(a&&a.matches&&a.matches("textarea.agent-input")),placeholder:a&&a.placeholder,len:a&&a.value?a.value.length:0,vis:document.visibilityState}})()`);
console.error("focused:", JSON.stringify(focus));
// Must be an agent composer: decision/question panels and modals have their own
// textareas with different event paths, which would mislabel the measurement.
if (!focus.composer) { console.error(`focused element is ${focus.tag ?? "nothing"}, not an agent composer (textarea.agent-input) — click into the composer first`); process.exit(2); }
// A minimized/backgrounded page throttles rAF and timers: the capture would
// measure visibility throttling, not typing responsiveness.
if (focus.vis !== "visible") { console.error(`page is ${focus.vis} — restore the window first`); process.exit(2); }

// 2. Arm the recorder. Snapshot the focused composer's draft + selection so the
// cleanup below restores it exactly rather than guessing which suffix was ours.
await evalIn(`(()=>{const a=document.activeElement;const P=window.__synProbe={t0:performance.now(),loafs:[],mut:0,keys:0,inputs:0,draft:{el:a,value:a.value,start:a.selectionStart,end:a.selectionEnd}};
  P.kh=()=>P.keys++; P.ih=()=>P.inputs++;
  document.addEventListener("keydown",P.kh,true); document.addEventListener("input",P.ih,true);
  P.mo=new MutationObserver(m=>P.mut+=m.length); P.mo.observe(document,{subtree:true,childList:true,characterData:true,attributes:true});
  P.po=new PerformanceObserver(l=>{for(const e of l.getEntries()){P.loafs.push({start:+(e.startTime-P.t0).toFixed(0),dur:+e.duration.toFixed(0),blocking:+e.blockingDuration.toFixed(0),render:e.renderStart?+(e.startTime+e.duration-e.renderStart).toFixed(0):0,styleLayout:e.styleAndLayoutStart?+(e.startTime+e.duration-e.styleAndLayoutStart).toFixed(0):0,scripts:(e.scripts||[]).map(s=>({inv:(s.invoker||"").slice(0,60),fn:(s.sourceFunctionName||"").slice(0,40),url:(s.sourceURL||"").replace(/^.*\\/(node_modules\\/\\.vite\\/deps|app)\\//,"$1/").slice(0,55),line:s.sourceCharPosition,dur:+s.duration.toFixed(1),fl:+s.forcedStyleAndLayoutDuration.toFixed(1)}))})}});
  P.po.observe({type:"long-animation-frame"});
  // rAF gap sampler
  P.gaps=[]; let last=performance.now(); P.raf=true; (function f(t){P.gaps.push(t-last); last=t; if(P.raf) requestAnimationFrame(f)})(last);
  return true})()`);

// 3. Type: keydown + char + keyup at kps, like OS key-repeat.
const n = Math.round(Number(secs) * Number(kps));
const interval = 1000 / Number(kps);
const keyCode = ch.toUpperCase().charCodeAt(0);
const tStart = Date.now();
for (let i = 0; i < n; i++) {
  const due = tStart + i * interval;
  const wait = due - Date.now(); if (wait > 0) await new Promise((r) => setTimeout(r, wait));
  // fire-and-forget so slow frames don't throttle our input rate (that's what the OS does)
  send("Input.dispatchKeyEvent", { type: "keyDown", key: ch, code: `Key${ch.toUpperCase()}`, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode, text: ch, unmodifiedText: ch, autoRepeat: i > 0 }).catch((e) => console.error(String(e)));
  send("Input.dispatchKeyEvent", { type: "keyUp", key: ch, code: `Key${ch.toUpperCase()}`, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode }).catch((e) => console.error(String(e)));
}
await new Promise((r) => setTimeout(r, 800)); // let the tail settle

// 4. Read out + disarm + restore the original draft.
const out = await evalIn(`(()=>{const P=window.__synProbe;P.raf=false;P.po.disconnect();P.mo.disconnect();document.removeEventListener("keydown",P.kh,true);document.removeEventListener("input",P.ih,true);
  const d=P.draft; const removed=d.el.value.length-d.value.length; d.el.value=d.value; d.el.setSelectionRange(d.start,d.end); d.el.dispatchEvent(new Event("input",{bubbles:true}));
  const L=P.loafs; const scr=l=>l.scripts.reduce((a,s)=>a+s.dur,0); const sum=f=>+L.reduce((a,l)=>a+f(l),0).toFixed(0);
  const q=(arr,p)=>{if(!arr.length)return null;const s=[...arr].sort((x,y)=>x-y);return +s[Math.min(s.length-1,Math.floor(p*s.length))].toFixed(1)};
  const byInv=new Map(); for(const l of L) for(const s of l.scripts){const k=s.inv+" | "+s.fn+" @ "+s.url+":"+s.line; const v=byInv.get(k)||{n:0,dur:0,fl:0}; v.n++; v.dur+=s.dur; v.fl+=s.fl; byInv.set(k,v)}
  return {elapsedMs:+(performance.now()-P.t0).toFixed(0),keydownsSeen:P.keys,inputsSeen:P.inputs,typedCharsRemoved:removed,mutations:P.mut,
    frames:P.gaps.length,rafGapMs:{p50:q(P.gaps,.5),p95:q(P.gaps,.95),max:q(P.gaps,1),over50:P.gaps.filter(g=>g>50).length},
    loaf:{count:L.length,totalMs:sum(l=>l.dur),blockingMs:sum(l=>l.blocking),scriptMs:sum(scr),forcedLayoutInScriptsMs:sum(l=>l.scripts.reduce((a,s)=>a+s.fl,0)),renderPhaseMs:sum(l=>l.render),ofWhichStyleLayoutMs:sum(l=>l.styleLayout),maxMs:L.length?Math.max(...L.map(l=>l.dur)):0},
    topScripts:[...byInv].map(([k,v])=>({k,n:v.n,ms:+v.dur.toFixed(0),forcedLayoutMs:+v.fl.toFixed(0)})).sort((a,b)=>b.ms-a.ms).slice(0,14),
    worst4:[...L].sort((a,b)=>b.dur-a.dur).slice(0,4).map(l=>({dur:l.dur,blocking:l.blocking,scriptMs:+scr(l).toFixed(0),renderMs:l.render,styleLayoutMs:l.styleLayout,top:[...l.scripts].sort((a,b)=>b.dur-a.dur).slice(0,3).map(s=>s.inv.slice(0,32)+"|"+s.fn+"@"+s.url.slice(-26)+" "+s.dur+"ms fl="+s.fl)}))}})()`);
ws.close();
console.log(JSON.stringify(out, null, 1));
