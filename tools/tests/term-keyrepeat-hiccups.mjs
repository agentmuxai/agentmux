#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// term-keyrepeat-hiccups.mjs — Diagnose typing "hiccups" during sustained
// key-repeat (hold a key ~15s) in a focused TERMINAL pane.
//
// Connects to the CEF renderer via CDP, installs an in-page monitor on EVERY
// shell window, and — while YOU hold a key in a focused term pane — records per
// window:
//   • rAF frame intervals   → dropped/janky frames (gaps > ~1.5 frames)
//   • longtask entries        → main-thread JS blocking ≥ 50 ms
//   • perf measures            → term-keypress / term-echo-render / term-raf-write
//   • focus + active-element sampling → PROVES the terminal was focused here
//   • xterm-rows MutationObserver    → PROVES the echo was actually rendering
//
// It then auto-selects the window where the terminal was active+echoing and
// classifies the hiccup: JS-blocking (longtasks), xterm write cost (big
// term-raf-write), or compositor/GPU stall (frame gaps with no JS).
//
// Usage:
//   node tools/tests/term-keyrepeat-hiccups.mjs [--cdp-port 9223] [--secs 18] [--json out.json]
//
// Steps: focus a terminal pane, run this, then HOLD a key when it says GO.

import { WebSocket } from "ws";

const argv = process.argv.slice(2);
const arg = (n, d) => { const i = argv.indexOf(n); return i !== -1 ? argv[i + 1] : d; };
const CDP_PORT = parseInt(arg("--cdp-port", "9223"), 10);
const SECS = parseInt(arg("--secs", "18"), 10);
const JSON_OUT = arg("--json", null);

async function listPages(port) {
    const r = await fetch(`http://127.0.0.1:${port}/json/list`).catch((e) => {
        throw new Error(`CDP unreachable on :${port} (${e.message}). Is the AgentMux instance running?`);
    });
    const targets = await r.json();
    return targets.filter((t) => t.type === "page" && t.webSocketDebuggerUrl);
}

function cdp(wsUrl) {
    const ws = new WebSocket(wsUrl, { maxPayload: 256 * 1024 * 1024 });
    const pending = new Map();
    let id = 1;
    ws.on("message", (raw) => {
        const m = JSON.parse(raw.toString());
        if (m.id && pending.has(m.id)) { const { resolve, reject } = pending.get(m.id); pending.delete(m.id); m.error ? reject(new Error(m.error.message)) : resolve(m.result); }
    });
    const ready = new Promise((res, rej) => { ws.on("open", res); ws.on("error", rej); });
    const send = (method, params = {}) => new Promise((resolve, reject) => { const i = id++; pending.set(i, { resolve, reject }); ws.send(JSON.stringify({ id: i, method, params })); });
    const evaluate = async (expression, awaitPromise = false) => {
        const r = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise });
        if (r.exceptionDetails) throw new Error("eval: " + (r.exceptionDetails.exception?.description || r.exceptionDetails.text));
        return r.result.value;
    };
    return { ready, send, evaluate, close: () => ws.close() };
}

const INSTALL = `(() => {
  if (window.__hic) { try { cancelAnimationFrame(window.__hic.raf); window.__hic.po && window.__hic.po.disconnect(); window.__hic.po2 && window.__hic.po2.disconnect(); clearInterval(window.__hic.fi); window.__hic.mo && window.__hic.mo.disconnect(); } catch(e){} }
  const h = { frames: [], t0: performance.now(), longtasks: [], measures: {}, focusSamples: 0, focusedSamples: 0, termActiveSamples: 0, rowMutations: 0 };
  let last = performance.now();
  const loop = (t) => { h.frames.push(+(t - last).toFixed(2)); last = t; h.raf = requestAnimationFrame(loop); };
  h.raf = requestAnimationFrame(loop);
  try { h.po = new PerformanceObserver((l) => { for (const e of l.getEntries()) h.longtasks.push({ s: +e.startTime.toFixed(1), d: +e.duration.toFixed(1) }); }); h.po.observe({ entryTypes: ["longtask"] }); } catch(e){}
  try { h.po2 = new PerformanceObserver((l) => { for (const e of l.getEntries()) { (h.measures[e.name] ||= []).push(+e.duration.toFixed(2)); } }); h.po2.observe({ entryTypes: ["measure"] }); } catch(e){}
  // Focus / active-element sampling: proves keystrokes were landing in a
  // terminal in THIS window during the recording.
  h.fi = setInterval(() => {
    h.focusSamples++;
    if (document.hasFocus()) h.focusedSamples++;
    const ae = document.activeElement;
    const cls = (ae && ae.className) || "";
    if (/xterm-helper-textarea|xterm/.test(cls) || (ae && ae.closest && ae.closest(".xterm"))) h.termActiveSamples++;
  }, 200);
  // MutationObserver on xterm rows = the echo actually rendering. Distinguishes
  // "frozen" from "rendering-but-janky".
  try {
    const rows = document.querySelector(".xterm-rows") || document.querySelector(".xterm");
    if (rows) { h.mo = new MutationObserver((m) => { h.rowMutations += m.length; }); h.mo.observe(rows, { childList: true, subtree: true, characterData: true }); }
  } catch(e){}
  window.__hic = h;
  return "started";
})()`;

const COLLECT = `(() => {
  const h = window.__hic; if (!h) return null;
  try { cancelAnimationFrame(h.raf); h.po && h.po.disconnect(); h.po2 && h.po2.disconnect(); clearInterval(h.fi); h.mo && h.mo.disconnect(); } catch(e){}
  const out = { durMs: +(performance.now() - h.t0).toFixed(0), frames: h.frames.slice(1), longtasks: h.longtasks, measures: h.measures,
    focusSamples: h.focusSamples, focusedSamples: h.focusedSamples, termActiveSamples: h.termActiveSamples, rowMutations: h.rowMutations };
  delete window.__hic;
  return out;
})()`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const pct = (sorted, p) => sorted.length ? sorted[Math.min(Math.floor(sorted.length * p), sorted.length - 1)] : 0;

function summarize(d) {
    const f = d.frames.filter((x) => x > 0).sort((a, b) => a - b);
    const n = f.length;
    const over = (t) => f.filter((x) => x > t).length;
    const fps = n ? (1000 * n / d.durMs).toFixed(1) : "0";
    console.log(`\n── Frame intervals (${n} frames over ${d.durMs} ms, ~${fps} fps observed) ──`);
    console.log(`  p50=${pct(f,0.5)}ms  p95=${pct(f,0.95)}ms  p99=${pct(f,0.99)}ms  max=${f[n-1]||0}ms`);
    console.log(`  jank frames:  >20ms=${over(20)}  >33ms=${over(33)}  >50ms=${over(50)}  >100ms=${over(100)}`);
    const lt = d.longtasks || [];
    const ltTotal = lt.reduce((a, b) => a + b.d, 0);
    console.log(`\n── Long tasks (main-thread JS ≥50ms) ──`);
    console.log(`  count=${lt.length}  totalBlocked=${ltTotal.toFixed(0)}ms  worst=${lt.length ? Math.max(...lt.map(x=>x.d)).toFixed(1) : 0}ms`);
    if (lt.length) console.log(`  durations(ms): ${lt.map(x=>x.d.toFixed(0)).sort((a,b)=>b-a).join(", ")}`);
    console.log(`\n── Perf measures (xterm marks) ──`);
    const names = Object.keys(d.measures || {}).sort();
    if (!names.length) console.log(`  (none captured)`);
    for (const name of names) {
        const a = d.measures[name].slice().sort((x, y) => x - y);
        console.log(`  ${name.padEnd(22)} n=${a.length} p50=${pct(a,0.5)}ms p95=${pct(a,0.95)}ms max=${a[a.length-1]}ms`);
    }
    console.log(`\n── Verdict ──`);
    const jank = over(33);
    const kp = (d.measures["term-keypress"] || []).length;
    if (jank === 0) console.log(`  No dropped frames during the burst. Hiccup may be sub-frame or input-side.`);
    else if (ltTotal > d.durMs * 0.05) console.log(`  Hiccups dominated by MAIN-THREAD JS (${ltTotal.toFixed(0)}ms blocked over ${d.durMs}ms). Fixable with scheduling/handler work — look at the worst longtask + term-raf-write.`);
    else console.log(`  ${jank} dropped frames with little/no JS blocking (${ltTotal.toFixed(0)}ms) → COMPOSITOR/GPU stalls (xterm WebGL atlas upload or layer recomposite), NOT JS. Needs render-path work, not scheduler work.`);
    console.log(`  (term-keypress marks this window: ${kp})`);
}

async function main() {
    const pages = await listPages(CDP_PORT);
    if (!pages.length) throw new Error(`No page targets on CDP :${CDP_PORT}.`);
    console.log(`Attaching to ${pages.length} page target(s)...`);
    const conns = [];
    for (const p of pages) {
        const c = cdp(p.webSocketDebuggerUrl);
        try { await c.ready; await c.send("Runtime.enable"); } catch (e) { console.log(`  ! connect failed: ${e.message}`); continue; }
        const id = (p.url.match(/clientId=([^&]+)/) || p.url.match(/ipc_port=(\d+)/) || [, p.url.slice(-12)])[1];
        let inst;
        try { inst = await c.evaluate(INSTALL); } catch (e) { console.log(`  ! install error on ${id}: ${e.message}`); c.close(); continue; }
        if (inst !== "started") { console.log(`  ! install rejected on ${id}`); c.close(); continue; }
        conns.push({ id, c });
    }
    if (!conns.length) throw new Error("could not install monitor on any window");
    console.log(`  monitoring ${conns.length} window(s): ${conns.map(x => x.id).join(", ")}`);

    console.log(`\n>>> GO — click into a TERMINAL pane and HOLD a key (e.g. 'o') now. Recording ${SECS}s... <<<`);
    for (let s = SECS; s > 0; s--) { process.stdout.write(`\r  ${s.toString().padStart(2)}s remaining   `); await sleep(1000); }
    process.stdout.write("\r  collecting...        \n");

    const all = [];
    for (const k of conns) {
        let data; try { data = await k.c.evaluate(COLLECT); } catch (e) { data = null; }
        k.c.close();
        if (data) all.push({ id: k.id, data });
    }
    if (!all.length) throw new Error("no data collected");

    // Active window = where the terminal was focused/active AND rows mutated.
    const score = (d) => (d.termActiveSamples || 0) * 1000 + (d.rowMutations || 0);
    all.sort((a, b) => score(b.data) - score(a.data));
    const active = all[0];
    const ad = active.data;

    console.log(`\n=== Focus attribution ===`);
    for (const w of all) {
        const d = w.data;
        const fp = d.focusSamples ? (100 * d.focusedSamples / d.focusSamples).toFixed(0) : "0";
        const tp = d.focusSamples ? (100 * d.termActiveSamples / d.focusSamples).toFixed(0) : "0";
        console.log(`  ${w.id}: window-focused ${fp}%, terminal-active ${tp}%, rowMutations=${d.rowMutations}, term-keypress=${(d.measures["term-keypress"]||[]).length}`);
    }

    const tActivePct = ad.focusSamples ? (100 * ad.termActiveSamples / ad.focusSamples) : 0;
    const valid = tActivePct >= 50 && ad.rowMutations > 0;
    console.log(`\n=== Active window: ${active.id}  [${valid ? "VALID" : "⚠ INVALID"}] ===`);
    if (!valid) {
        console.log(`  ⚠️  The terminal was not focused/echoing for most of the hold in any window`);
        console.log(`     (terminal-active ${tActivePct.toFixed(0)}%, rowMutations ${ad.rowMutations}).`);
        console.log(`     → click directly INTO a terminal pane, HOLD the key the whole ${SECS}s, don't click away. Re-run.`);
    }
    summarize(ad);
    if (JSON_OUT) { (await import("fs")).writeFileSync(JSON_OUT, JSON.stringify(all)); console.log(`\nraw → ${JSON_OUT}`); }
}

main().catch((e) => { console.error("\nError:", e.message); process.exit(1); });
