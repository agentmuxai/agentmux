// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window-resize frame probe (macOS first; the driver is the host's own
// `set_window_rect`, the counterpart of the SetWindowPos stepping used for
// docs/analysis/ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md §2).
//
//   node scripts/perf/macos-paint/resize-frames.mjs [--steps 80] [--step-px 10] [--interval-ms 16] [--label x] [--runs 3]
//
// Page side: every requestAnimationFrame gap, every `resize` event (and how long
// until the next frame after it), every long-animation-frame entry.
// Driver side (Node, so it adds no work to the page's main thread): POSTs
// set_window_rect to the host IPC every interval, shrinking then growing the
// width by `step-px` DIP per step.
import os from "node:os";
import { connectPage } from "../../ui-screenshots/lib/cdp-client.mjs";

const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i < 0 ? d : (process.argv[i + 1] ?? true); };
const steps = +arg("steps", 80), stepPx = +arg("step-px", 10), intervalMs = +arg("interval-ms", 16);
const label = String(arg("label", "run")), runs = +arg("runs", 1), port = +arg("port", 9223);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const { session: page } = await connectPage(port, process.env.PROBE_SEL ?? "window_transparent");
const ev = async (expression) => {
    const r = await page.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 400));
    return r.result.value;
};
const creds = await ev(`({ port: window.__AGENTMUX_IPC_PORT__, token: window.__AGENTMUX_IPC_TOKEN__, origin: performance.timeOrigin })`);
const ipc = async (cmd, args) => {
    const t = performance.now();
    const res = await fetch(`http://127.0.0.1:${creds.port}/ipc`, {
        method: "POST", headers: { "Content-Type": "application/json", Authorization: `Bearer ${creds.token}` },
        body: JSON.stringify({ cmd, args }),
    });
    const j = await res.json();
    if (!j.success) throw new Error(j.error);
    return { data: j.data, ms: performance.now() - t };
};

const INSTALL = `(() => {
  if (window.__fp?.po) window.__fp.po.disconnect();
  const fp = (window.__fp = { frames: [], resizes: [], r2f: [], loaf: [], run: true });
  const loop = (t) => { if (!fp.run) return; fp.frames.push(t); requestAnimationFrame(loop); };
  requestAnimationFrame(loop);
  window.addEventListener("resize", () => { const t = performance.now(); fp.resizes.push([t, innerWidth]); requestAnimationFrame((ft) => fp.r2f.push(performance.now() - t)); });
  try { fp.po = new PerformanceObserver((l) => { for (const e of l.getEntries()) fp.loaf.push({ s: e.startTime, d: e.duration, b: e.blockingDuration, sl: e.styleAndLayoutStart, r: e.renderStart }); }); fp.po.observe({ type: "long-animation-frame", buffered: false }); } catch (e) { fp.loafErr = String(e); }
  return true;
})()`;

const q = (a, p) => (a.length ? a[Math.min(a.length - 1, Math.floor(p * a.length))] : 0);
const results = [];
const brief = process.argv.includes("--brief");
for (let run = 0; run < runs; run++) {
    const { data: r0 } = await ipc("get_window_rect", { label: process.env.PROBE_LABEL ?? "main" });
    await ev(INSTALL);
    await sleep(300);
    const passive = +arg("passive", 0);
    const ps = await ev(`performance.now()`);
    const half = Math.floor(steps / 2), callMs = [], t0 = performance.now();
    if (passive > 0) { console.error(`recording ${passive}s: drag the window edge with the mouse now...`); await sleep(passive * 1000); }
    for (let i = 0; i < (passive > 0 ? 0 : steps); i++) {
        const a = performance.now();
        const dx = i < half ? -(i + 1) * stepPx : -(steps - i - 1) * stepPx;
        const { ms } = await ipc("set_window_rect", { label: process.env.PROBE_LABEL ?? "main", x: r0.x, y: r0.y, width: r0.width + dx, height: r0.height });
        callMs.push(ms);
        const wait = intervalMs - (performance.now() - a);
        if (wait > 0) await sleep(wait);
    }
    const driverMs = performance.now() - t0;
    const pe = await ev(`performance.now()`);
    if (!(passive > 0)) await ipc("set_window_rect", { label: process.env.PROBE_LABEL ?? "main", x: r0.x, y: r0.y, width: r0.width, height: r0.height });
    await sleep(500);
    const origin2 = await ev(`performance.timeOrigin`);
    if (origin2 !== creds.origin) { console.error("PAGE RELOADED during the run; discarding"); continue; }
    const d = await ev(`(() => { window.__fp.run = false; const f = window.__fp; return { frames: f.frames, resizes: f.resizes, r2f: f.r2f, loaf: f.loaf, loafErr: f.loafErr, w: innerWidth, dpr: devicePixelRatio }; })()`);
    // Only what happened while the window was being stepped (+100 ms for the tail).
    const fr = d.frames.filter((t) => t >= ps && t <= pe + 100), gaps = fr.slice(1).map((t, i) => t - fr[i]).sort((a, b) => a - b);
    d.resizes = d.resizes.filter((r) => r[0] >= ps && r[0] <= pe + 100);
    d.loaf = d.loaf.filter((e) => e.s >= ps && e.s <= pe + 100);
    const r2f = [...d.r2f].sort((a, b) => a - b), cm = [...callMs].sort((a, b) => a - b);
    results.push({
        label, run: run + 1, dpr: d.dpr, load1m: +os.loadavg()[0].toFixed(1), stepsSent: steps, driverMs: Math.round(driverMs),
        idealFrames: Math.round(driverMs / 16.7), frames: fr.length,
        gapMs: { p50: +q(gaps, 0.5).toFixed(1), p90: +q(gaps, 0.9).toFixed(1), p99: +q(gaps, 0.99).toFixed(1), max: +(gaps.at(-1) ?? 0).toFixed(1) },
        resizeEventsSeen: d.resizes.length, distinctWidths: new Set(d.resizes.map((r) => r[1])).size,
        resizeToNextFrameMs: { p50: +q(r2f, 0.5).toFixed(1), p90: +q(r2f, 0.9).toFixed(1), max: +(r2f.at(-1) ?? 0).toFixed(1) },
        longFrames: d.loaf.length, longTotalMs: Math.round(d.loaf.reduce((s, e) => s + e.d, 0)),
        longMaxMs: Math.round(Math.max(0, ...d.loaf.map((e) => e.d))),
        longStyleLayoutMs: Math.round(d.loaf.reduce((s, e) => s + Math.max(0, (e.r || 0) - (e.sl || 0)), 0)),
        ipcCallMs: { p50: +q(cm, 0.5).toFixed(1), p90: +q(cm, 0.9).toFixed(1), max: +(cm.at(-1) ?? 0).toFixed(1) },
        finalWidthOk: d.w === r0.width,
    });
}
if (brief) {
    for (const r of results) console.log(`${r.label} #${r.run} load=${r.load1m} frames=${r.frames}/${r.idealFrames} gap p50/p90/p99/max=${r.gapMs.p50}/${r.gapMs.p90}/${r.gapMs.p99}/${r.gapMs.max} sizesSeen=${r.distinctWidths}/${r.stepsSent} long=${r.longFrames} (${r.longTotalMs}ms, max ${r.longMaxMs}) ipc p50=${r.ipcCallMs.p50}`);
} else console.log(JSON.stringify(results.length === 1 ? results[0] : results, null, 1));
process.exit(0);
