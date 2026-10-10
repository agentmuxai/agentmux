// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Records a window resize end to end: page frames, long frames, how far the page's layout width
// trails the native window, and how far right-edge browser-pane windows trail the main window.
//   node drag.mjs <cef pid> --mode armed     wait for a real mouse drag; stop 1.5 s after it ends
//   node drag.mjs <cef pid> --mode scripted  step the width with set_window_rect (--steps, --step-px)
import os from "node:os";
import { spawn } from "node:child_process";
import { connectPage } from "../../ui-screenshots/lib/cdp-client.mjs";
import { parseSamples, paneEdgeLag, pageTrail } from "./drag-analysis.mjs";
const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i < 0 ? d : process.argv[i + 1]; };
const pid = process.argv[2], mode = arg("mode", "armed"), label = arg("label", mode);
const steps = +arg("steps", 80), stepPx = +arg("step-px", 4), maxWait = +arg("max-wait", 180);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const { session: page } = await connectPage(9223, "window_transparent");
const ev = async (e) => { const r = await page.send("Runtime.evaluate", { expression: e, awaitPromise: true, returnByValue: true }); if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 300)); return r.result.value; };
const env = await ev(`(() => {
  if (window.__dr?.po) window.__dr.po.disconnect();
  const d = (window.__dr = { frames: [], resizes: [], loaf: [], run: true });
  const loop = (t) => { if (!d.run) return; d.frames.push([t, innerWidth]); requestAnimationFrame(loop); }; requestAnimationFrame(loop);
  window.addEventListener("resize", () => d.resizes.push([performance.now(), innerWidth]));
  d.po = new PerformanceObserver((l) => { for (const e of l.getEntries()) d.loaf.push([e.startTime, e.duration]); }); d.po.observe({ type: "long-animation-frame" });
  return { W: innerWidth, H: innerHeight, origin: performance.timeOrigin, port: window.__AGENTMUX_IPC_PORT__, token: window.__AGENTMUX_IPC_TOKEN__ };
})()`);
const ipc = async (cmd, args) => (await (await fetch(`http://127.0.0.1:${env.port}/ipc`, { method: "POST", headers: { "Content-Type": "application/json", Authorization: `Bearer ${env.token}` }, body: JSON.stringify({ cmd, args }) })).json());
const samp = spawn("./panesampler", [pid, String(maxWait + 30)], { stdio: ["ignore", "pipe", "inherit"] });
let raw = ""; samp.stdout.on("data", (b) => (raw += b));
await sleep(400);
if (mode === "scripted") {
    const r0 = (await ipc("get_window_rect", { label: "main" })).data;
    for (let i = 0; i < steps; i++) { const a = performance.now(); const dx = i < steps / 2 ? -(i + 1) * stepPx : -(steps - i - 1) * stepPx; await ipc("set_window_rect", { label: "main", x: r0.x, y: r0.y, width: r0.width + dx, height: r0.height }); const w = 16 - (performance.now() - a); if (w > 0) await sleep(w); }
    await ipc("set_window_rect", { label: "main", ...r0 });
    await sleep(1500);
} else {
    console.log(`ARMED (${label}): drag the window's RIGHT edge out and back a few times, then let go. Waiting up to ${maxWait}s...`);
    const t0 = Date.now(); let started = false;
    for (;;) {
        const [n, last, now] = await ev(`[window.__dr.resizes.length, window.__dr.resizes.at(-1)?.[0] ?? 0, performance.now()]`);
        if (n > 0 && !started) { started = true; console.log("  drag detected, recording..."); }
        if (started && now - last > 1500) break;
        if (!started && Date.now() - t0 > maxWait * 1000) { console.log("  no drag seen; giving up"); break; }
        await sleep(150);
    }
}
samp.kill("SIGTERM"); await new Promise((r) => samp.on("close", r));
const d = await ev(`(() => { window.__dr.run = false; return { frames: window.__dr.frames, resizes: window.__dr.resizes, loaf: window.__dr.loaf }; })()`);
if (!d.resizes.length) { console.log("no resize recorded"); process.exit(1); }
const ep = (t) => env.origin + t;
const from = ep(d.resizes[0][0]) - 100, to = ep(d.resizes.at(-1)[0]) + 300;
// Frames are judged over the drag itself (first to last resize); the trail and pane checks keep the tail.
const f0 = ep(d.resizes[0][0]), f1 = ep(d.resizes.at(-1)[0]) + 17;
const fr = d.frames.filter(([t]) => ep(t) >= f0 && ep(t) <= f1).map(([t]) => t);
const gaps = fr.slice(1).map((t, i) => t - fr[i]).sort((a, b) => a - b);
const q = (a, p) => +(a.length ? a[Math.min(a.length - 1, Math.floor(p * a.length))] : 0).toFixed(1);
const loaf = d.loaf.filter(([s]) => ep(s) >= f0 && ep(s) <= f1);
const samples = parseSamples(raw);
const hint = { W: env.W, H: env.H };
const pane = paneEdgeLag(samples, { from, to, hint });
const trail = pageTrail(samples, d.frames.map(([t, w]) => [ep(t), w]), { from, to, hint });
const durMs = to - from;
console.log(`${label}: drag ${Math.round(f1 - f0)} ms, load ${os.loadavg()[0].toFixed(1)} | page frames ${fr.length}/${Math.round((f1 - f0) / 16.7)} gap p50/p90/p99 ${q(gaps, .5)}/${q(gaps, .9)}/${q(gaps, .99)} ms | long frames ${loaf.length} (${Math.round(loaf.reduce((s, [, x]) => s + x, 0))} ms) | resize events ${d.resizes.length}`);
console.log(`  page width vs native window width: off by >2pt in ${trail.offSharePct}% of ${trail.samples} samples; p50/p90/p99/max ${trail.trailPx.p50}/${trail.trailPx.p90}/${trail.trailPx.p99}/${trail.trailPx.max} pt`);
if (pane && pane.edgePanes) console.log(`  right-edge browser panes (${pane.edgePanes}): off by >2pt in ${pane.offSharePct}% of samples; p50/p90/p99/max ${pane.devPx.p50}/${pane.devPx.p90}/${pane.devPx.p99}/${pane.devPx.max} pt; last off-position ${pane.settleMs} ms after the window's last size change | window sizes seen ${pane.mainSizes}`);
process.exit(0);
