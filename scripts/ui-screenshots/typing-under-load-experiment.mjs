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
const send = (method, params = {}) => new Promise((res, rej) => {
  // A send after the socket closed would never get a response (and failAll
  // has already run), so refuse it outright; a failed write rejects too.
  if (ws.readyState !== WebSocket.OPEN) return rej(new Error(`${method}: CDP socket is not open`));
  const i = ++id; pending.set(i, (j) => (j.error ? rej(new Error(`${method}: ${j.error.message}`)) : res(j)));
  ws.send(JSON.stringify({ id: i, method, params }), (err) => { if (err) { pending.delete(i); rej(new Error(`${method}: ${err.message}`)); } });
});
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
// Hidden pages suspend rAF and throttle timers, so a window during which the
// page was hidden at any moment measures throttling, not typing. The check
// above is only the start: every window re-checks on arm and on read, and a
// listener catches a hide-and-restore that happens entirely inside a window.
const HIDDEN_DURING = (w) => w.hiddenDuring || w.visibilityAtRead !== "visible";

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
const ARM = `(()=>{if(document.visibilityState!=="visible")throw new Error("page is "+document.visibilityState+" at window start — restore the window");
  const P=window.__exp={t0:performance.now(),loafs:[],mut:0,keys:0,gaps:[],stray:0,hiddenDuring:false};
  P.vh=()=>{if(document.visibilityState!=="visible")P.hiddenDuring=true}; document.addEventListener("visibilitychange",P.vh);
  // During a typed window, a synthetic key whose target is not the captured
  // composer (focus moved, the footer remounted on a tab switch) is swallowed
  // before it can type into anything else, and counted so the window aborts.
  const d=window.__expDraft; P.kh=e=>{P.keys++; if(d&&e.target!==d.el){e.preventDefault();e.stopImmediatePropagation();P.stray++}}; document.addEventListener("keydown",P.kh,true);
  P.mo=new MutationObserver(m=>P.mut+=m.length); P.mo.observe(document,{subtree:true,childList:true,characterData:true,attributes:true});
  P.po=new PerformanceObserver(P.poCb=l=>{for(const e of l.getEntries()){P.loafs.push({start:+(e.startTime-P.t0).toFixed(0),dur:+e.duration.toFixed(0),blocking:+e.blockingDuration.toFixed(0),render:e.renderStart?+(e.startTime+e.duration-e.renderStart).toFixed(0):0,styleLayout:e.styleAndLayoutStart?+(e.startTime+e.duration-e.styleAndLayoutStart).toFixed(0):0,scripts:(e.scripts||[]).map(s=>({inv:(s.invoker||"").slice(0,60),fn:(s.sourceFunctionName||"").slice(0,40),url:(s.sourceURL||"").replace(/^.*\\/(node_modules\\/\\.vite\\/deps|app)\\//,"$1/").slice(0,55),line:s.sourceCharPosition,dur:+s.duration.toFixed(1),fl:+s.forcedStyleAndLayoutDuration.toFixed(1)}))})}});
  P.po.observe({type:"long-animation-frame"});
  let last=performance.now(); P.raf=true; (function f(t){P.gaps.push(t-last); last=t; if(P.raf) requestAnimationFrame(f)})(last); return true})()`;
const READ = `(()=>{const P=window.__exp;P.raf=false;P.poCb({getEntries:()=>P.po.takeRecords()});P.mut+=P.mo.takeRecords().length;P.po.disconnect();P.mo.disconnect();document.removeEventListener("keydown",P.kh,true);document.removeEventListener("visibilitychange",P.vh);
  const L=P.loafs; const scr=l=>l.scripts.reduce((a,s)=>a+s.dur,0); const sum=f=>+L.reduce((a,l)=>a+f(l),0).toFixed(0);
  const q=(arr,p)=>{if(!arr.length)return null;const s=[...arr].sort((x,y)=>x-y);return +s[Math.min(s.length-1,Math.floor(p*s.length))].toFixed(1)};
  const byInv=new Map(); for(const l of L) for(const s of l.scripts){const k=s.inv+" | "+s.fn+" @ "+s.url+":"+s.line; const v=byInv.get(k)||{n:0,dur:0,fl:0}; v.n++; v.dur+=s.dur; v.fl+=s.fl; byInv.set(k,v)}
  const d=window.__expDraft;
  return {elapsedMs:+(performance.now()-P.t0).toFixed(0),keydowns:P.keys,strayKeys:P.stray,hiddenDuring:P.hiddenDuring,visibilityAtRead:document.visibilityState,composerDetached:!!(d&&!d.el.isConnected),mutations:P.mut,frames:P.gaps.length,gaps:P.gaps.map(g=>+g.toFixed(1)),loafDurs:L.map(l=>l.dur),
    rafGapMs:{p50:q(P.gaps,.5),p95:q(P.gaps,.95),max:q(P.gaps,1),over50:P.gaps.filter(g=>g>50).length},
    loaf:{count:L.length,totalMs:sum(l=>l.dur),blockingMs:sum(l=>l.blocking),scriptMs:sum(scr),forcedLayoutInScriptsMs:sum(l=>l.scripts.reduce((a,s)=>a+s.fl,0)),renderPhaseMs:sum(l=>l.render),ofWhichStyleLayoutMs:sum(l=>l.styleLayout),maxMs:L.length?Math.max(...L.map(l=>l.dur)):0},
    allScripts:[...byInv].map(([k,v])=>({k,n:v.n,ms:+v.dur.toFixed(0),forcedLayoutMs:+v.fl.toFixed(0)})),
    topScripts:[...byInv].map(([k,v])=>({k,n:v.n,ms:+v.dur.toFixed(0),forcedLayoutMs:+v.fl.toFixed(0)})).sort((a,b)=>b.ms-a.ms).slice(0,16),
    worst3:[...L].sort((a,b)=>b.dur-a.dur).slice(0,3).map(l=>({dur:l.dur,blocking:l.blocking,scriptMs:+scr(l).toFixed(0),renderMs:l.render,styleLayoutMs:l.styleLayout,top:[...l.scripts].sort((a,b)=>b.dur-a.dur).slice(0,3).map(s=>s.inv.slice(0,32)+"|"+s.fn+"@"+s.url.slice(-26)+" "+s.dur+"ms fl="+s.fl)}))}})()`;

// One window of exactly `secs`. B types into composer `typeIdx`; if it can't be
// focused, keystrokes would land wherever focus happens to be and the window
// would measure the wrong workload — so that aborts the run instead.
const kc = ch.toUpperCase().charCodeAt(0), interval = 1000 / kps;
// Restores the composer exactly; used by runWindow's finally and by signals.
//
// Through the LIVE composer, not just the captured element. Every synthetic key
// went through AgentFooter.handleInput, which persists the value per block
// (composerDrafts); if the footer remounted during the window (an ordinary
// pane-stack tab switch), the captured element is detached and the new footer
// restored the synthetic text from that map, so writing to the detached
// element would fix nothing. So: the captured element if still connected, else
// the block's current composer (found via the block frame's data-blockid); the
// bubbled input event runs handleInput, which persists the original draft back.
// If the block has no composer mounted, wait up to 30 s for it to come back;
// failing that, report it with the draft text so nothing is lost silently.
const RESTORE = `(async()=>{const d=window.__expDraft;if(!d)return {ok:true,how:"nothing to restore"};
  const write=el=>{el.value=d.value;try{el.setSelectionRange(d.start,d.end)}catch{}el.dispatchEvent(new Event("input",{bubbles:true}))};
  const done=how=>{delete window.__expDraft;return {ok:true,how}};
  if(d.el.isConnected){write(d.el);return done("captured composer")}
  const find=()=>d.blockId?document.querySelector('[data-blockid="'+CSS.escape(d.blockId)+'"] textarea.agent-input'):null;
  for(let i=0;i<300;i++){const el=find();if(el){write(el);return done("remounted composer")}await new Promise(r=>setTimeout(r,100))}
  return {ok:false,blockId:d.blockId,draft:d.value}})()`;
const restoreDraft = async () => {
  const r = await evalIn(RESTORE);
  if (r && r.ok === false) {
    console.error(`\nCOULD NOT RESTORE the composer draft of block ${r.blockId}: its composer was not mounted within 30 s.`);
    console.error(`The composer may show synthetic "${ch}" characters. Your original draft was:\n---\n${r.draft}\n---`);
    return false;
  }
  if (r && r.how === "remounted composer") log("restored the draft through the remounted composer");
  return true;
};
// The restore's input event runs AgentFooter.handleInput, which schedules a
// scroll via requestAnimationFrame — let that land before the next window arms,
// or its work is measured in (and credited to) the wrong condition.
const SETTLE = `new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>setTimeout(r,50))))`;
const restoreAndExit = async (code) => { let ok = true; try { ok = await restoreDraft(); } catch { ok = false; } process.exit(ok ? code : 4); };
process.once("SIGINT", () => restoreAndExit(130));
process.once("SIGTERM", () => restoreAndExit(143));
async function runWindow(typed) {
  if (typed) {
    if (!(await focusComposerAt(typeIdx))) throw new Error(`could not focus composer ${typeIdx}`);
    // Snapshot draft + selection + owning block so cleanup restores it exactly,
    // even through a remounted composer (see RESTORE).
    await evalIn(`(()=>{const a=document.activeElement;window.__expDraft={el:a,value:a.value,start:a.selectionStart,end:a.selectionEnd,blockId:a.closest("[data-blockid]")?.getAttribute("data-blockid")??null};return true})()`);
  }
  await evalIn(ARM);
  const t0 = Date.now();
  try {
    if (typed) {
      for (let i = 0; i < secs * kps; i++) {
        const w = t0 + i * interval - Date.now(); if (w > 0) await sleep(w);
        // Once a second, stop typing as soon as the composer is gone (the key
        // guard already swallows strays; this ends the window promptly).
        if (i > 0 && i % kps === 0 && !(await evalIn(`!!window.__expDraft&&window.__expDraft.el.isConnected&&document.activeElement===window.__expDraft.el`))) {
          throw new Error("the composer lost focus or was remounted during the typed window");
        }
        send("Input.dispatchKeyEvent", { type: "keyDown", key: ch, code: `Key${ch.toUpperCase()}`, windowsVirtualKeyCode: kc, nativeVirtualKeyCode: kc, text: ch, unmodifiedText: ch, autoRepeat: i > 0 }).catch((e) => console.error(String(e)));
        send("Input.dispatchKeyEvent", { type: "keyUp", key: ch, code: `Key${ch.toUpperCase()}`, windowsVirtualKeyCode: kc, nativeVirtualKeyCode: kc }).catch((e) => console.error(String(e)));
      }
    }
    const remaining = t0 + secs * 1000 - Date.now();
    if (remaining > 0) await sleep(remaining);
    const r = await evalIn(READ);
    if (HIDDEN_DURING(r)) throw new Error("the page was hidden during this window (rAF/timers throttled); restore the window and rerun");
    if (r.strayKeys > 0 || r.composerDetached) throw new Error(`typing left the composer during the window (${r.strayKeys} stray key(s)${r.composerDetached ? ", composer remounted" : ""})`);
    return r;
  } finally {
    if (typed) {
      const ok = await restoreDraft().catch(() => false);
      await evalIn(SETTLE).catch(() => {});
      if (!ok) process.exitCode = 4;
    }
  }
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
