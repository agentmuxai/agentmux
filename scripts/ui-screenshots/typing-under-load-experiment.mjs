// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Hands-free A/B: same streaming load, LoAF-attributed frames WITHOUT typing
// (A) vs WITH synthetic key-repeat typing (B). Optionally kicks off the load by
// sending a prompt into every EMPTY agent composer in the page first.
//
//   node scripts/ui-screenshots/typing-under-load-experiment.mjs <port> <targetIdSubstr> \
//        [--send "<prompt>"] [--ramp 40] [--secs 15] [--kps 20] [--cycles 2] --typeInto <paneIndex|agentName>
//
// Windows are counterbalanced (A B, then B A, ...), each exactly `secs` long,
// because the load itself drifts — streams grow and agents finish — so a fixed
// A-then-B order would credit that drift to typing. Totals are pooled per
// condition; `windows` shows each window so drift is visible.
import WebSocket from "ws";

const args = process.argv.slice(2);
const port = args[0] ?? "9223", targetSel = args[1] ?? "";
const opt = (k, d) => { const i = args.indexOf(k); return i > 0 ? args[i + 1] : d; };
const prompt = opt("--send", null), ramp = Number(opt("--ramp", "40")), secs = Number(opt("--secs", "15"));
const kps = Number(opt("--kps", "20")), typeInto = opt("--typeInto", null), cycles = Number(opt("--cycles", "2")), ch = "f";
if (typeInto == null) { console.error("--typeInto <paneIndex|agentName> is required"); process.exit(1); }

const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page" && (t.id.includes(targetSel) || t.url.includes(targetSel)));
if (!page) { console.error("no page target"); process.exit(1); }
const ws = new WebSocket(page.webSocketDebuggerUrl, { perMessageDeflate: false, maxPayload: 1 << 28 });
await new Promise((r, e) => { ws.once("open", r); ws.once("error", e); });
let id = 0; const pending = new Map();
ws.on("message", (m) => { const j = JSON.parse(m); if (j.id && pending.has(j.id)) { pending.get(j.id)(j); pending.delete(j.id); } });
const send = (method, params = {}) => new Promise((res, rej) => { const i = ++id; pending.set(i, (j) => (j.error ? rej(new Error(`${method}: ${j.error.message}`)) : res(j))); ws.send(JSON.stringify({ id: i, method, params })); });
// A dropped CDP connection (target reload/close) sends no responses; reject
// everything still waiting instead of hanging forever.
const failAll = (why) => { for (const f of pending.values()) f({ error: { message: why } }); pending.clear(); };
ws.on("close", () => failAll("CDP socket closed"));
ws.on("error", (e) => failAll(`CDP socket error: ${e.message}`));
const evalIn = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description || JSON.stringify(r.result.exceptionDetails));
  return r.result?.result?.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (...a) => console.error(new Date().toISOString().slice(11, 19), ...a);

const vis = await evalIn(`document.visibilityState`);
if (vis !== "visible") { console.error(`page is ${vis} — restore the window first`); process.exit(2); }

// Composers in document order. A pane is addressed by its index, or by an agent
// name that must match EXACTLY one "Send message to <name>..." placeholder —
// never a substring (`Maka` must not select `Makashi`). Placeholders turn into
// next-prompt suggestions once an agent has run, so the index is the reliable form.
// Only agent composers (`textarea.agent-input`) — decision/question panels and
// modals have textareas too — captured ONCE as elements, so a textarea mounted
// or removed later can't shift an index onto a different pane.
const composers = await evalIn(`(()=>{window.__expComposers=[...document.querySelectorAll("textarea.agent-input")];return window.__expComposers.map((t,i)=>({i,name:(t.placeholder.match(/^Send message to (.+)\\.\\.\\.$/)||[])[1]||null,placeholder:t.placeholder.slice(0,40),hasDraft:t.value.length>0}))})()`);
log("composers:", composers.map((c) => `${c.i}:${c.name ?? `"${c.placeholder}"`}${c.hasDraft ? " (draft)" : ""}`).join(", "));
const resolveComposer = (sel) => {
  if (/^\d+$/.test(String(sel))) return composers[Number(sel)] ? Number(sel) : null;
  const hits = composers.filter((c) => c.name === sel);
  return hits.length === 1 ? hits[0].i : null;
};
// Refuses an element that has left the DOM since capture (pane closed/remounted).
const focusComposerAt = (i) => evalIn(`(()=>{const ta=window.__expComposers[${i}];if(!ta||!ta.isConnected)return false;ta.focus();return document.activeElement===ta})()`);
const typeIdx = resolveComposer(typeInto);
if (typeIdx == null) { console.error(`--typeInto ${typeInto} matches no single composer (see list above)`); process.exit(3); }

// --- optional: send the prompt into every EMPTY composer ---
if (prompt) {
  for (const c of composers) {
    // Input.insertText edits whatever is there and Enter submits the whole
    // textarea, which the composer then clears — never do that to a real draft.
    const hasDraft = await evalIn(`(window.__expComposers[${c.i}]?.value.length ?? 1) > 0`);
    if (hasDraft) { log(`skipped composer ${c.i}: it holds an unsent draft`); continue; }
    if (!(await focusComposerAt(c.i))) { log("could not focus composer", c.i); continue; }
    const n = c.i;
    await send("Input.insertText", { text: prompt });
    await sleep(150);
    await send("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13, text: "\r" });
    await send("Input.dispatchKeyEvent", { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 });
    await sleep(300);
    log("sent to", n);
  }
  log(`ramping ${ramp}s for streams to get heavy ...`);
  await sleep(ramp * 1000);
}

// --- LoAF recorder (shared) ---
const ARM = `(()=>{const P=window.__exp={t0:performance.now(),loafs:[],mut:0,keys:0,gaps:[]};
  P.kh=()=>P.keys++; document.addEventListener("keydown",P.kh,true);
  P.mo=new MutationObserver(m=>P.mut+=m.length); P.mo.observe(document,{subtree:true,childList:true,characterData:true,attributes:true});
  P.po=new PerformanceObserver(P.poCb=l=>{for(const e of l.getEntries()){P.loafs.push({start:+(e.startTime-P.t0).toFixed(0),dur:+e.duration.toFixed(0),blocking:+e.blockingDuration.toFixed(0),render:e.renderStart?+(e.startTime+e.duration-e.renderStart).toFixed(0):0,styleLayout:e.styleAndLayoutStart?+(e.startTime+e.duration-e.styleAndLayoutStart).toFixed(0):0,scripts:(e.scripts||[]).map(s=>({inv:(s.invoker||"").slice(0,60),fn:(s.sourceFunctionName||"").slice(0,40),url:(s.sourceURL||"").replace(/^.*\\/(node_modules\\/\\.vite\\/deps|app)\\//,"$1/").slice(0,55),line:s.sourceCharPosition,dur:+s.duration.toFixed(1),fl:+s.forcedStyleAndLayoutDuration.toFixed(1)}))})}});
  P.po.observe({type:"long-animation-frame"});
  let last=performance.now(); P.raf=true; (function f(t){P.gaps.push(t-last); last=t; if(P.raf) requestAnimationFrame(f)})(last); return true})()`;
const READ = `(()=>{const P=window.__exp;P.raf=false;P.poCb({getEntries:()=>P.po.takeRecords()});P.mut+=P.mo.takeRecords().length;P.po.disconnect();P.mo.disconnect();document.removeEventListener("keydown",P.kh,true);
  const L=P.loafs; const scr=l=>l.scripts.reduce((a,s)=>a+s.dur,0); const sum=f=>+L.reduce((a,l)=>a+f(l),0).toFixed(0);
  const q=(arr,p)=>{if(!arr.length)return null;const s=[...arr].sort((x,y)=>x-y);return +s[Math.min(s.length-1,Math.floor(p*s.length))].toFixed(1)};
  const byInv=new Map(); for(const l of L) for(const s of l.scripts){const k=s.inv+" | "+s.fn+" @ "+s.url+":"+s.line; const v=byInv.get(k)||{n:0,dur:0,fl:0}; v.n++; v.dur+=s.dur; v.fl+=s.fl; byInv.set(k,v)}
  return {elapsedMs:+(performance.now()-P.t0).toFixed(0),keydowns:P.keys,mutations:P.mut,frames:P.gaps.length,gaps:P.gaps.map(g=>+g.toFixed(1)),loafDurs:L.map(l=>l.dur),
    rafGapMs:{p50:q(P.gaps,.5),p95:q(P.gaps,.95),max:q(P.gaps,1),over50:P.gaps.filter(g=>g>50).length},
    loaf:{count:L.length,totalMs:sum(l=>l.dur),blockingMs:sum(l=>l.blocking),scriptMs:sum(scr),forcedLayoutInScriptsMs:sum(l=>l.scripts.reduce((a,s)=>a+s.fl,0)),renderPhaseMs:sum(l=>l.render),ofWhichStyleLayoutMs:sum(l=>l.styleLayout),maxMs:L.length?Math.max(...L.map(l=>l.dur)):0},
    allScripts:[...byInv].map(([k,v])=>({k,n:v.n,ms:+v.dur.toFixed(0),forcedLayoutMs:+v.fl.toFixed(0)})),
    topScripts:[...byInv].map(([k,v])=>({k,n:v.n,ms:+v.dur.toFixed(0),forcedLayoutMs:+v.fl.toFixed(0)})).sort((a,b)=>b.ms-a.ms).slice(0,16),
    worst3:[...L].sort((a,b)=>b.dur-a.dur).slice(0,3).map(l=>({dur:l.dur,blocking:l.blocking,scriptMs:+scr(l).toFixed(0),renderMs:l.render,styleLayoutMs:l.styleLayout,top:[...l.scripts].sort((a,b)=>b.dur-a.dur).slice(0,3).map(s=>s.inv.slice(0,32)+"|"+s.fn+"@"+s.url.slice(-26)+" "+s.dur+"ms fl="+s.fl)}))}})()`;

// One window of exactly `secs`. B types into composer `typeIdx`; if it can't be
// focused, keystrokes would land wherever focus happens to be and the window
// would measure the wrong workload — so that aborts the run instead.
const kc = ch.toUpperCase().charCodeAt(0), interval = 1000 / kps;
async function runWindow(typed) {
  if (typed) {
    if (!(await focusComposerAt(typeIdx))) throw new Error(`could not focus composer ${typeIdx}`);
    // Snapshot draft + selection so cleanup restores it exactly.
    await evalIn(`(()=>{const a=document.activeElement;window.__expDraft={el:a,value:a.value,start:a.selectionStart,end:a.selectionEnd};return true})()`);
  }
  await evalIn(ARM);
  const t0 = Date.now();
  if (typed) {
    for (let i = 0; i < secs * kps; i++) {
      const w = t0 + i * interval - Date.now(); if (w > 0) await sleep(w);
      send("Input.dispatchKeyEvent", { type: "keyDown", key: ch, code: `Key${ch.toUpperCase()}`, windowsVirtualKeyCode: kc, nativeVirtualKeyCode: kc, text: ch, unmodifiedText: ch, autoRepeat: i > 0 }).catch((e) => console.error(String(e)));
      send("Input.dispatchKeyEvent", { type: "keyUp", key: ch, code: `Key${ch.toUpperCase()}`, windowsVirtualKeyCode: kc, nativeVirtualKeyCode: kc }).catch((e) => console.error(String(e)));
    }
  }
  const remaining = t0 + secs * 1000 - Date.now();
  if (remaining > 0) await sleep(remaining);
  const r = await evalIn(READ);
  if (typed) await evalIn(`(()=>{const d=window.__expDraft;if(d&&d.el){d.el.value=d.value;d.el.setSelectionRange(d.start,d.end);d.el.dispatchEvent(new Event("input",{bubbles:true}))}delete window.__expDraft;return true})()`);
  return r;
}

// Pool a condition's windows: totals summed, percentiles over the pooled samples.
function pool(ws_) {
  const q = (arr, p) => { if (!arr.length) return null; const s = [...arr].sort((x, y) => x - y); return +s[Math.min(s.length - 1, Math.floor(p * s.length))].toFixed(1); };
  const sum = (f) => ws_.reduce((a, w) => a + f(w), 0);
  const gaps = ws_.flatMap((w) => w.gaps), durs = ws_.flatMap((w) => w.loafDurs);
  const byInv = new Map();
  // Complete per-window totals, so a script that is expensive in every window
  // but never top-16 in any single one still ranks correctly once pooled.
  for (const w of ws_) for (const s of w.allScripts) { const v = byInv.get(s.k) || { k: s.k, n: 0, ms: 0, forcedLayoutMs: 0 }; v.n += s.n; v.ms += s.ms; v.forcedLayoutMs += s.forcedLayoutMs; byInv.set(s.k, v); }
  return {
    windows: ws_.length, totalMs: sum((w) => w.elapsedMs), keydowns: sum((w) => w.keydowns), mutations: sum((w) => w.mutations), frames: gaps.length,
    rafGapMs: { p50: q(gaps, .5), p95: q(gaps, .95), max: q(gaps, 1), over50: gaps.filter((g) => g > 50).length },
    loaf: { count: durs.length, totalMs: sum((w) => w.loaf.totalMs), blockingMs: sum((w) => w.loaf.blockingMs), scriptMs: sum((w) => w.loaf.scriptMs), forcedLayoutInScriptsMs: sum((w) => w.loaf.forcedLayoutInScriptsMs), maxMs: durs.length ? Math.max(...durs) : 0 },
    topScripts: [...byInv.values()].sort((a, b) => b.ms - a.ms).slice(0, 12),
  };
}

const order = []; for (let c = 0; c < cycles; c++) order.push(...(c % 2 === 0 ? ["A", "B"] : ["B", "A"]));
const got = { A: [], B: [] }, windows = [];
try {
  for (const [k, label] of order.entries()) {
    log(`${label}${k}) ${secs}s streaming${label === "B" ? ` + synthetic typing into composer ${typeIdx} at ${kps}/s` : ", NO typing"}`);
    const r = await runWindow(label === "B");
    got[label].push(r);
    windows.push({ window: k, label, frames: r.frames, mutations: r.mutations, keydowns: r.keydowns, loafCount: r.loaf.count, loafTotalMs: r.loaf.totalMs });
  }
} catch (e) {
  log(`aborted: ${e.message}`);
  ws.close();
  console.log(JSON.stringify({ aborted: e.message, windows }, null, 1));
  process.exit(3);
}
ws.close();
console.log(JSON.stringify({ A_streaming_only: pool(got.A), B_streaming_plus_typing: pool(got.B), windows }, null, 1));
